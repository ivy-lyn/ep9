//! Tests of the megamorphic stub cache (`stub_cache.rs`, M4-P3 IC2) and of the string method
//! loads that `ic.rs` caches from M4-P3 on: megamorphic loads, stores and adds through the
//! cache, the operation in the key (load, 0x24 store, 0x25 define), the validity cell of a
//! prototype case, the flush at every GC, and `KEY_STRING` prototype cases.
//!
//! The harness is `tests::ic` (`T`, one site per function, every dispatch expansion, with and
//! without GC stress, eager feedback). The objects that make a site megamorphic are rooted
//! ([`T::keep_all`]) up to the last assertion that needs the site megamorphic: case keys are
//! weak, and a shape that dies frees its case.

use super::ic::{
    MASK, SLOT, Site, T, at_capacity, below_a_pad, collect_after_a_pad, each, grow_adds, int, key,
    object_prototype, object_with, own,
};
use super::{Asm, isolate, meta, unit};
use ep9_heap::GlobalHandle;

use crate::frame::UNDEFINED;
use crate::ic::{self, KEY_STRING, State, hk, smi};
use crate::object::{self, attr};
use crate::property;
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

/// Five objects with `x` (= 10..14) in five shapes, under `p`. The caller roots them
/// ([`T::keep_all`]) while a state or an entry needs their shapes.
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
        let kept = t.keep_all(&objs);
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
        t.release_all(kept);
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
        let kept = t.keep_all(&objs);
        for &q in &objs {
            t.ok(0, q, UNDEFINED);
            t.ok(1, q, int(1));
        }
        assert_eq!(
            (t.state(0), t.state(1)),
            (State::Megamorphic, State::Megamorphic)
        );
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
        t.release_all(kept);
    });
}

#[test]
fn the_stub_cache_is_empty_after_every_gc() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x")]);
        let p = object_prototype(t.rt());
        let objs = five(&t, p);
        let kept = t.keep_all(&objs);
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
        t.release_all(kept);
    });
}

/// Makes the load site megamorphic, then loads `x` from a new object in a shape no other
/// object has, so that an entry keyed on that shape exists. Returns the masked addresses of
/// the object and its shape. The five objects are rooted only up to the entry check.
#[inline(never)]
fn made_an_entry(t: &mut T) -> [usize; 2] {
    let p = object_prototype(t.rt());
    let objs = five(t, p);
    let kept = t.keep_all(&objs);
    for &o in &objs {
        t.ok(0, o, UNDEFINED);
    }
    let o = object_with(t.rt(), p, &[("lone", int(0)), ("x", int(9))]);
    assert_eq!(t.ok(0, o, UNDEFINED), int(9));
    assert!(entry(t, Op::Load, o, "x").is_some());
    t.release_all(kept);
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
        let kept = t.keep_all(&objs);
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
        t.release_all(kept);
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
        let kept = t.keep_all(&objs);
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
        t.release_all(kept);
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
        let kept_o = t.keep(o);
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
        // Rooted: the state counts all five shapes after the first object's last use (on
        // x86-64 release that object's frame slot was reused, and under GC stress it died:
        // `Polymorphic(4)`, run 40733504045).
        let kept = t.keep_all(&lens);
        for (i, &l) in lens.iter().enumerate() {
            assert_eq!(t.ok(1, l, UNDEFINED), int(i as i32));
        }
        assert_eq!(t.state(1), State::Megamorphic);
        let s = string::new_string(t.rt(), "abc");
        assert_eq!(t.ok(1, s, UNDEFINED), int(3));
        assert_eq!(t.ok(1, s, UNDEFINED), int(3));
        t.release_all(kept);
        t.release(kept_o);
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

// ---- the probe of the GetNamedProperty handler (M7-P1 NAMEDLOAD) -------------------------------

/// The number of loads that the probe in the cold body of `GetNamedProperty` answered
/// (`ic::get_named_cold`).
fn handler_hits() -> u32 {
    ic::HANDLER_PROBE_HITS.with(|c| c.get())
}

/// Makes site 0 (`a.<name>`) of `t` megamorphic with five objects under `p` that have `name`
/// as their second property. The caller roots the result while the state matters.
fn megamorphic(t: &mut T, p: u64, name: &str) -> [u64; 5] {
    let pads = ["a", "b", "c", "d", "e"];
    let objs = core::array::from_fn(|i| {
        object_with(t.rt(), p, &[(pads[i], int(0)), (name, int(10 + i as i32))])
    });
    for &o in &objs {
        t.ok(0, o, UNDEFINED);
    }
    assert_eq!(t.state(0), State::Megamorphic);
    objs
}

/// Loads `o.<name>` at site 0 twice: the first load fills the entry (the runtime call), the
/// second one must come from the probe of the handler. Without GC stress the count proves it;
/// under GC stress a collection may empty the table first, so only the values are checked.
/// Returns the handler kind of the entry the first load made (`ic::describe_handler`).
fn fill_then_hit(t: &mut T, o: u64, name: &str, want: u64, stress: bool) -> i32 {
    assert_eq!(t.ok(0, o, UNDEFINED), want);
    let kind = entry(t, Op::Load, o, name).map(|h| ic::describe_handler(h).0);
    let before = handler_hits();
    assert_eq!(t.ok(0, o, UNDEFINED), want);
    if !stress {
        assert_eq!(
            handler_hits(),
            before + 1,
            "the handler's probe answered `{name}`"
        );
    }
    kind.unwrap_or(0)
}

#[test]
fn the_get_named_handler_answers_megamorphic_loads_from_the_stub_cache() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut t = T::new(c, &[Site::Get("x"), Site::Get("m"), Site::Get("length")]);
        let op = object_prototype(t.rt());
        let objs = megamorphic(&mut t, op, "x");
        let kept = t.keep_all(&objs);
        // An own field in the object.
        let o = object_with(t.rt(), op, &[("f", int(0)), ("x", int(3))]);
        let k = fill_then_hit(&mut t, o, "x", int(3), stress);
        assert!(stress || k == smi::FIELD_IN_OBJECT, "kind {k}");
        // An own field in the SLOTS cell: past the in-object capacity of a new object.
        let names: Vec<String> = (0..40).map(|i| format!("p{i}")).collect();
        let mut props: Vec<(&str, u64)> = names.iter().map(|n| (n.as_str(), int(0))).collect();
        props.push(("x", int(4)));
        let big = object_with(t.rt(), op, &props);
        let k = fill_then_hit(&mut t, big, "x", int(4), stress);
        assert!(stress || k == smi::FIELD_OUT_OF_LINE, "kind {k}");
        // A data property of a prototype (PROTO_LOAD) and a missing one (MISSING).
        let base = object_with(t.rt(), op, &[("x", int(5))]);
        let child = object_with(t.rt(), base, &[("g", int(0))]);
        let k = fill_then_hit(&mut t, child, "x", int(5), stress);
        assert!(stress || k == PROTO_LOAD, "kind {k}");
        let none = object_with(t.rt(), op, &[("h", int(0))]);
        let k = fill_then_hit(&mut t, none, "x", UNDEFINED, stress);
        assert!(stress || k == hk::MISSING | 0x100, "kind {k}");
        // Not an object at the megamorphic site: a string, a number and a symbol take the
        // runtime call, whose generic load gives undefined.
        let before = handler_hits();
        let s = string::new_string(t.rt(), "abc");
        assert_eq!(t.ok(0, s, UNDEFINED), UNDEFINED);
        assert_eq!(t.ok(0, int(7), UNDEFINED), UNDEFINED);
        let sym = symbol::new_symbol(t.rt(), UNDEFINED);
        assert_eq!(t.ok(0, sym, UNDEFINED), UNDEFINED);
        assert_eq!(handler_hits(), before);
        t.release_all(kept);
        // An array's `length` at a megamorphic `length` site (ARRAY_LENGTH).
        let lens: [u64; 5] = core::array::from_fn(|i| {
            object_with(
                t.rt(),
                op,
                &[(["a", "b", "c", "d", "e"][i], int(0)), ("length", int(1))],
            )
        });
        let kept = t.keep_all(&lens);
        for &l in &lens {
            t.ok(2, l, UNDEFINED);
        }
        assert_eq!(t.state(2), State::Megamorphic);
        let a = object::new_array(t.rt(), 0);
        assert_eq!(t.ok(2, a, UNDEFINED), int(0));
        let before = handler_hits();
        assert_eq!(t.ok(2, a, UNDEFINED), int(0));
        if !stress {
            assert_eq!(
                entry(&t, Op::Load, a, "length").map(|h| ic::describe_handler(h).0),
                Some(smi::ARRAY_LENGTH)
            );
            assert_eq!(handler_hits(), before + 1);
        }
        t.release_all(kept);
    });
}

#[test]
fn a_probe_hit_in_the_handler_still_checks_the_validity_cell() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut t = T::new(c, &[Site::Get("m")]);
        let op = object_prototype(t.rt());
        let base = object_with(t.rt(), op, &[("m", int(1))]);
        let mid = object_with(t.rt(), base, &[]);
        let objs = megamorphic(&mut t, mid, "q");
        let kept = t.keep_all(&objs);
        let o = object_with(t.rt(), mid, &[("lone", int(0))]);
        let k = fill_then_hit(&mut t, o, "m", int(1), stress);
        assert!(stress || k == PROTO_LOAD, "kind {k}");
        // A shadowing `m` on `mid`: the entry's cell goes invalid. The probe finds the entry,
        // its handler misses, and the runtime call finds the new `m`.
        let before = handler_hits();
        props::own_define(t.rt(), mid, key(t.rt(), "m"), int(3), attr::DEFAULT);
        assert_eq!(t.ok(0, o, UNDEFINED), int(3));
        assert_eq!(handler_hits(), before);
        // The runtime call put a new case in the entry; the handler answers from it.
        fill_then_hit(&mut t, o, "m", int(3), stress);
        // A value store to the holder is not a change: the next hit reads the new value.
        assert!(props::own_replace(t.rt(), mid, key(t.rt(), "m"), int(4)));
        assert_eq!(t.ok(0, o, UNDEFINED), int(4));
        t.release_all(kept);
    });
}

#[test]
fn a_super_load_does_not_use_the_probe_of_the_get_named_handler() {
    each(|c| {
        let mut t = T::new(c, &[Site::Super("x")]);
        let op = object_prototype(t.rt());
        // super.x with the home object `h_i`: the IC is keyed on the lookup start, the
        // prototype of `h_i`.
        let pads = ["a", "b", "c", "d", "e", "f"];
        let starts: [u64; 6] = core::array::from_fn(|i| {
            object_with(t.rt(), op, &[(pads[i], int(0)), ("x", int(i as i32))])
        });
        let homes: [u64; 6] = core::array::from_fn(|i| object_with(t.rt(), starts[i], &[]));
        let kept = t.keep_all(&starts);
        let kept_h = t.keep_all(&homes);
        let this = object::new_plain_object(t.rt());
        for (i, &h) in homes.iter().enumerate() {
            assert_eq!(t.ok(0, h, this), int(i as i32));
        }
        assert_eq!(t.state(0), State::Megamorphic);
        let before = handler_hits();
        for _ in 0..2 {
            assert_eq!(t.ok(0, homes[5], this), int(5));
        }
        assert_eq!(handler_hits(), before);
        t.release_all(kept_h);
        t.release_all(kept);
    });
}

#[test]
fn a_new_stub_cache_is_empty() {
    each(|c| {
        let t = T::new(c, &[Site::Get("x")]);
        assert_eq!(t.rt().stub_cache.len(), 0);
        assert_eq!(stub_cache::ENTRIES, 8192);
    });
}

// ---- the probe of the store handlers and the add with growth (M8-P0 TS-STORE) ----------------

/// The number of stores that the probe in the cold bodies of `SetNamedProperty` and
/// `DefineNamedOwnProperty` answered (`ic::handler_mega_store`).
fn store_hits() -> u32 {
    ic::HANDLER_STORE_HITS.with(|c| c.get())
}

/// Makes the store site `i` of `t` megamorphic with five objects under `p` that get `name` (a
/// field store). The caller roots the result while the state matters.
fn megamorphic_store(t: &mut T, i: usize, p: u64, name: &str) -> [u64; 5] {
    let pads = ["a", "b", "c", "d", "e"];
    let objs = core::array::from_fn(|j| {
        object_with(t.rt(), p, &[(pads[j], int(0)), (name, int(10 + j as i32))])
    });
    for &o in &objs {
        t.ok(i, o, int(1));
    }
    assert_eq!(t.state(i), State::Megamorphic);
    objs
}

/// Stores `v` into `o.<name>` at site `i` twice, with `o2` (same shape as `o`) the second
/// time: the first store fills the entry (the runtime call), the second one must come from the
/// probe of the handler. Without GC stress the count proves it; under GC stress a collection
/// may empty the table first, so only the values are checked.
fn store_fill_then_hit(t: &mut T, i: usize, o: u64, o2: u64, name: &str, stress: bool) {
    assert_eq!(id(o), id(o2));
    t.ok(i, o, int(7));
    let before = store_hits();
    t.ok(i, o2, int(8));
    assert_eq!(own(t.rt(), o, name), Some(int(7)));
    assert_eq!(own(t.rt(), o2, name), Some(int(8)));
    if !stress {
        assert_eq!(
            store_hits(),
            before + 1,
            "the handler's probe stored `{name}`"
        );
    }
}

#[test]
fn the_set_named_handlers_answer_megamorphic_stores_from_the_stub_cache() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut t = T::new(
            c,
            &[
                Site::Set("x", false),
                Site::Define("x"),
                Site::Set("x", true),
            ],
        );
        let op = object_prototype(t.rt());
        let mut kept = Vec::new();
        for i in [0, 1, 2] {
            let objs = megamorphic_store(&mut t, i, op, "x");
            kept.push(t.keep_all(&objs));
        }
        for i in [0, 1, 2] {
            // An own field in the object, and an add with room (in the object).
            let f = |t: &T, pad: &str| object_with(t.rt(), op, &[(pad, int(0)), ("x", int(3))]);
            let (o, o2) = (f(&t, "f"), f(&t, "f"));
            store_fill_then_hit(&mut t, i, o, o2, "x", stress);
            let a = |t: &T| object_with(t.rt(), op, &[("g", int(0))]);
            let (o, o2) = (a(&t), a(&t));
            store_fill_then_hit(&mut t, i, o, o2, "x", stress);
            assert_eq!(id(o), id(o2), "both took the add");
        }
        // Not an object at the megamorphic site: the runtime call (a sloppy store to a
        // primitive is ignored; a strict one throws).
        let before = store_hits();
        let s = string::new_string(t.rt(), "abc");
        t.ok(0, s, int(1));
        t.ok(0, int(7), int(1));
        assert!(t.call(2, int(7), int(1)).is_err());
        assert_eq!(store_hits(), before);
        for k in kept {
            t.release_all(k);
        }
    });
}

/// A probe hit of a 0x24 add still checks the validity cell: a setter that a prototype gets
/// after the entry was made runs (the probe misses, the runtime call takes the generic path).
#[test]
fn a_store_probe_hit_still_checks_the_validity_cell() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut t = T::new(c, &[Site::Set("x", false)]);
        let op = object_prototype(t.rt());
        let p = object_with(t.rt(), op, &[("pp", int(0))]);
        let objs = megamorphic_store(&mut t, 0, p, "q");
        let kept = t.keep_all(&objs);
        let a = |t: &T| object_with(t.rt(), p, &[("lone", int(0))]);
        let (o, o2) = (a(&t), a(&t));
        store_fill_then_hit(&mut t, 0, o, o2, "x", stress);
        // A read-only `x` on the prototype: the add case of the entry no longer applies.
        props::own_define(
            t.rt(),
            p,
            key(t.rt(), "x"),
            int(5),
            attr::ENUMERABLE | attr::CONFIGURABLE,
        );
        let o3 = a(&t);
        let before = store_hits();
        t.ok(0, o3, int(9));
        assert_eq!(
            own(t.rt(), o3, "x"),
            None,
            "the read-only `x` blocks the add"
        );
        assert_eq!(store_hits(), before);
        t.release_all(kept);
    });
}

/// An add whose stub cache entry has no room in the SLOTS cell: the probe of the handler
/// misses, and the runtime call grows the SLOTS cell and stores through the entry (`ic::mega_store`,
/// `ic::grow_add`).
#[test]
fn a_megamorphic_add_with_no_room_grows_through_the_stub_cache_entry() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut t = T::new(c, &[Site::Set("x", false), Site::Define("x")]);
        let op = object_prototype(t.rt());
        let mut kept = Vec::new();
        for i in [0, 1] {
            let objs = megamorphic_store(&mut t, i, op, "x");
            kept.push(t.keep_all(&objs));
        }
        for i in [0, 1] {
            let first = at_capacity(t.rt(), op);
            let from = id(first);
            t.ok(i, first, int(1));
            let to = id(first);
            let op_ = if i == 0 { Op::Store } else { Op::Define };
            let made = stub_cache::probe(t.rt(), op_, from, key(t.rt(), "x")).is_some();
            let (hits, grown) = (store_hits(), grow_adds());
            let q = at_capacity(t.rt(), op);
            assert_eq!(id(q), from);
            t.ok(i, q, int(2));
            assert_eq!(own(t.rt(), q, "x"), Some(int(2)));
            assert_eq!(id(q), to);
            assert_eq!(store_hits(), hits, "the handler's probe cannot grow");
            if !stress {
                assert!(made, "the first add made the entry");
                assert_eq!(grow_adds(), grown + 1, "the runtime call grew it");
            }
        }
        for k in kept {
            t.release_all(k);
        }
    });
}

// ---- the probe in the GetNamedProperty fast body (M10 LEVER-12 NAMED-MISS) ---------------------

/// The number of loads that the probe in the fast body of `GetNamedProperty` answered
/// (`ic::load_or_mega`; [`handler_hits`] counts them too).
fn fast_hits() -> u32 {
    ic::FAST_PROBE_HITS.with(|c| c.get())
}

/// `n` objects under `p`, each with its own first property (so each has its own shape) and
/// then `name` = 100 + i, each rooted as it is made (a `Vec` is not on the stack, so the
/// conservative scan does not see it): the objects and their handles.
fn shapes(t: &T, p: u64, name: &str, n: usize) -> (Vec<u64>, Vec<GlobalHandle>) {
    (0..n)
        .map(|i| {
            let pad = format!("pad{i}");
            let o = object_with(
                t.rt(),
                p,
                &[(pad.as_str(), int(0)), (name, int(100 + i as i32))],
            );
            (o, t.keep(o))
        })
        .unzip()
}

/// Loads `o.<name>` at site 0 of `t`: `want`, and a hit in the fast body only if the stub
/// cache held an entry for it (without GC stress; under GC stress a collection may empty the
/// table first, so only the value is checked). True if the fast body answered.
fn load_counted(t: &mut T, o: u64, name: &str, want: u64, stress: bool) -> bool {
    let held = entry(t, Op::Load, o, name).is_some();
    let f = fast_hits();
    assert_eq!(t.ok(0, o, UNDEFINED), want);
    let hit = fast_hits() == f + 1;
    assert!(stress || hit || fast_hits() == f);
    assert!(stress || held || !hit, "a fast hit with no entry");
    hit
}

#[test]
fn the_fast_body_answers_a_megamorphic_site_with_forty_shapes() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut t = T::new(c, &[Site::Get("x")]);
        let op = object_prototype(t.rt());
        let (objs, kept) = shapes(&t, op, "x", 40);
        // Each shape: the first load fills its entry (the runtime call), the second one is a
        // hit in the fast body (not the cold body: both counts move by one).
        for (i, &o) in objs.iter().enumerate() {
            let want = int(100 + i as i32);
            assert_eq!(t.ok(0, o, UNDEFINED), want);
            let (h, f) = (handler_hits(), fast_hits());
            assert_eq!(t.ok(0, o, UNDEFINED), want);
            if !stress && i >= 4 {
                assert_eq!((handler_hits(), fast_hits()), (h + 1, f + 1), "shape {i}");
            }
        }
        assert_eq!(t.state(0), State::Megamorphic);
        // Again, in another order: an entry that a conflict replaced misses and is filled
        // again; every other load is a fast-body hit. The values are right in both cases.
        for (i, &o) in objs.iter().enumerate().rev() {
            let held = entry(&t, Op::Load, o, "x").is_some();
            let f = fast_hits();
            assert_eq!(t.ok(0, o, UNDEFINED), int(100 + i as i32));
            if !stress {
                assert_eq!(fast_hits(), f + u32::from(held), "shape {i}");
            }
        }
        t.release_all(kept);
    });
}

#[test]
fn a_shape_the_stub_cache_does_not_hold_goes_to_the_cold_path() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut t = T::new(c, &[Site::Get("x")]);
        let op = object_prototype(t.rt());
        let objs = megamorphic(&mut t, op, "x");
        let kept = t.keep_all(&objs);
        // A new shape: no entry, so the fast body and the cold body both miss and the runtime
        // call answers (and fills the entry).
        let o = object_with(t.rt(), op, &[("new", int(0)), ("x", int(7))]);
        assert!(entry(&t, Op::Load, o, "x").is_none());
        let (h, f) = (handler_hits(), fast_hits());
        assert_eq!(t.ok(0, o, UNDEFINED), int(7));
        assert_eq!((handler_hits(), fast_hits()), (h, f));
        let (h, f) = (handler_hits(), fast_hits());
        assert_eq!(t.ok(0, o, UNDEFINED), int(7));
        if !stress {
            assert_eq!((handler_hits(), fast_hits()), (h + 1, f + 1));
        }
        // A GC empties the table: the next load misses in both bodies, and the runtime call
        // fills the entry again.
        t.collect();
        assert!(entry(&t, Op::Load, o, "x").is_none());
        let f = fast_hits();
        assert_eq!(t.ok(0, o, UNDEFINED), int(7));
        if !stress {
            assert_eq!(fast_hits(), f);
        }
        fill_then_hit(&mut t, o, "x", int(7), stress);
        // A value store to the field is not a shape change: the fast hit reads the new value.
        assert!(props::own_replace(t.rt(), o, key(t.rt(), "x"), int(8)));
        assert_eq!(t.ok(0, o, UNDEFINED), int(8));
        // Not an object: a BigInt (ShapeId 0, as a string and a symbol) misses in the fast
        // body and takes the generic load (which throws in v0: no BigInt.prototype).
        let b = crate::bigint::alloc_literal(t.rt(), false, &[5]);
        let (h, f) = (handler_hits(), fast_hits());
        assert!(t.call(0, b, UNDEFINED).is_err());
        assert!(t.call(0, b, UNDEFINED).is_err());
        assert_eq!((handler_hits(), fast_hits()), (h, f));
        t.release_all(kept);
    });
}

#[test]
fn a_dictionary_receiver_at_a_megamorphic_site_takes_the_generic_path() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x")]);
        let op = object_prototype(t.rt());
        let objs = megamorphic(&mut t, op, "x");
        let kept = t.keep_all(&objs);
        // A dictionary-mode object (a delete that is not the last, OM-10): no entry for its
        // shape, so every load is the generic one and reads the current value.
        let d = object_with(t.rt(), op, &[("pad", int(0)), ("x", int(7))]);
        let pad = key(t.rt(), "pad");
        assert!(property::delete(&mut t.iso.cx, d, pad, true).unwrap());
        assert!(shape::is_dictionary(shape::shape_of(d)));
        let (h, f) = (handler_hits(), fast_hits());
        for _ in 0..3 {
            assert_eq!(t.ok(0, d, UNDEFINED), int(7));
        }
        assert_eq!((handler_hits(), fast_hits()), (h, f));
        assert!(entry(&t, Op::Load, d, "x").is_none());
        props::own_define(t.rt(), d, key(t.rt(), "x"), int(9), attr::DEFAULT);
        assert_eq!(t.ok(0, d, UNDEFINED), int(9));
        // A dictionary prototype holds `x` for a fast receiver: no entry either.
        let dp = object_with(t.rt(), op, &[("pad", int(0)), ("x", int(11))]);
        assert!(property::delete(&mut t.iso.cx, dp, pad, true).unwrap());
        let child = object_with(t.rt(), dp, &[("c", int(0))]);
        let f = fast_hits();
        for _ in 0..3 {
            assert_eq!(t.ok(0, child, UNDEFINED), int(11));
        }
        assert_eq!(fast_hits(), f);
        props::own_define(t.rt(), dp, key(t.rt(), "x"), int(12), attr::DEFAULT);
        assert_eq!(t.ok(0, child, UNDEFINED), int(12));
        t.release_all(kept);
    });
}

#[test]
fn a_prototype_change_makes_the_fast_probe_hit_miss() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut t = T::new(c, &[Site::Get("m")]);
        let op = object_prototype(t.rt());
        let base = object_with(t.rt(), op, &[("m", int(1))]);
        let other = object_with(t.rt(), op, &[("m", int(9))]);
        let mid = object_with(t.rt(), base, &[]);
        let kept_p = t.keep_all(&[base, other, mid]);
        let objs = megamorphic(&mut t, mid, "q");
        let kept = t.keep_all(&objs);
        let o = object_with(t.rt(), mid, &[("lone", int(0))]);
        let k = fill_then_hit(&mut t, o, "m", int(1), stress);
        assert!(stress || k == PROTO_LOAD, "kind {k}");
        let f = fast_hits();
        assert_eq!(t.ok(0, o, UNDEFINED), int(1));
        if !stress {
            assert_eq!(fast_hits(), f + 1);
        }
        // `mid`'s prototype changes: the entry's validity cell goes invalid, so the fast probe
        // finds the entry but its handler misses, and the runtime call finds `other.m`. The
        // loads after it hit in the fast body only if the miss path put a new entry there.
        assert!(property::set_prototype_of(t.rt(), mid, other));
        assert!(stress || entry(&t, Op::Load, o, "m").is_some());
        assert!(!load_counted(&mut t, o, "m", int(9), stress));
        for _ in 0..2 {
            assert!(load_counted(&mut t, o, "m", int(9), stress) || stress);
        }
        // `m` goes away from the holder `other`: undefined, never a stale 9; then the MISSING
        // case hits in the fast body. `m` comes back: never a stale undefined.
        assert!(props::own_delete(t.rt(), other, key(t.rt(), "m")).is_some());
        assert!(!load_counted(&mut t, o, "m", UNDEFINED, stress));
        assert!(load_counted(&mut t, o, "m", UNDEFINED, stress) || stress);
        props::own_define(t.rt(), other, key(t.rt(), "m"), int(4), attr::DEFAULT);
        assert!(!load_counted(&mut t, o, "m", int(4), stress));
        assert!(load_counted(&mut t, o, "m", int(4), stress) || stress);
        // The receiver's own prototype changes: it turns dictionary (no entry is ever made
        // for it), and every load is the generic one, which follows the new chain.
        assert!(property::set_prototype_of(t.rt(), o, base));
        assert!(shape::is_dictionary(shape::shape_of(o)));
        for _ in 0..2 {
            assert!(!load_counted(&mut t, o, "m", int(1), stress));
        }
        t.release_all(kept);
        t.release_all(kept_p);
    });
}

/// Function `f0(a)`: `return a.x` with the name constant at index 300, so the load is a
/// `Wide` GetNamedProperty.
fn wide_site() -> ep9_bytecode::VerifiedUnit {
    let mut s = Asm::script(0);
    let k = s.b.function_constant(ep9_bytecode::FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.sta_global("f0");
    s.b.lda_undefined().return_();
    let mut b = ep9_bytecode::FunctionBuilder::new(meta("f0", 2, 0, 0, 0));
    let a = b.parameter(0);
    let slot = b.feedback_slot(ep9_bytecode::FeedbackSlotKind::LoadNamed);
    for i in 0..300 {
        b.number_constant(f64::from(i) + 0.5);
    }
    let k = b.string_constant("x");
    assert!(k.index() >= 256);
    b.get_named_property(a, k, slot).return_();
    unit("", vec![s.finish(), b.finish().unwrap()])
}

#[test]
fn a_wide_get_named_probes_in_its_cold_body_only() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut iso = isolate(c);
        iso.with_cx_checkpoint(|cx| cx.run_script(wide_site(), "wide.js").map(|_| ()))
            .unwrap();
        let mut t = T { iso };
        let op = object_prototype(t.rt());
        let objs = megamorphic(&mut t, op, "x");
        let kept = t.keep_all(&objs);
        let tries = fast_tries();
        let o = object_with(t.rt(), op, &[("w", int(0)), ("x", int(5))]);
        assert_eq!(t.ok(0, o, UNDEFINED), int(5));
        let (h, f) = (handler_hits(), fast_hits());
        assert_eq!(t.ok(0, o, UNDEFINED), int(5));
        if !stress {
            assert_eq!((handler_hits(), fast_hits()), (h + 1, f));
        }
        // The scale test of the fast body (Codex nm-r1 F2): no probe was tried at all.
        assert_eq!(fast_tries(), tries);
        t.release_all(kept);
    });
}

// ---- the probes of one megamorphic miss (M10 LEVER-12 Codex nm-r1 F1) -------------------------

/// The stub cache probes (`stub_cache::PROBES`) of one load `f0(o)` of `t`, which gives `want`.
fn probes_of(t: &mut T, o: u64, want: u64) -> u32 {
    let p = stub_cache::PROBES.with(|c| c.get());
    assert_eq!(t.ok(0, o, UNDEFINED), want);
    stub_cache::PROBES.with(|c| c.get()) - p
}

/// A dictionary-mode object under `p` with `x` = 7 (a delete that is not the last, OM-10):
/// no stub cache entry is ever made for its shape, so each load of `x` at a megamorphic site
/// misses. The caller roots it.
fn dictionary_with_x(t: &mut T, p: u64) -> u64 {
    let d = object_with(t.rt(), p, &[("pad", int(0)), ("x", int(7))]);
    let pad = key(t.rt(), "pad");
    assert!(property::delete(&mut t.iso.cx, d, pad, true).unwrap());
    assert!(shape::is_dictionary(shape::shape_of(d)));
    d
}

/// Codex nm-r1 F1: a megamorphic miss of a one-byte GetNamedProperty probes the stub cache
/// twice, in the fast body (`ic::load_or_mega`) and in the runtime call (`ic::mega_load`), as
/// the cold-body probe and the runtime call did before M10 LEVER-12: its cold body does not
/// probe the same entry again (`ic::get_named_cold_at`). A `Wide` one probes in its cold body
/// and in the runtime call (twice too). A hit in either body probes once. GC does not probe,
/// so the miss counts hold under GC stress too.
#[test]
fn a_megamorphic_miss_probes_the_stub_cache_twice() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut t = T::new(c, &[Site::Get("x")]);
        let op = object_prototype(t.rt());
        let objs = megamorphic(&mut t, op, "x");
        let kept = t.keep_all(&objs);
        let d = dictionary_with_x(&mut t, op);
        let kept_d = t.keep(d);
        for _ in 0..3 {
            assert_eq!(probes_of(&mut t, d, int(7)), 2);
        }
        // A new shape: the first load misses (two probes) and fills the entry; the next one
        // hits in the fast body (one probe).
        let o = object_with(t.rt(), op, &[("new", int(0)), ("x", int(8))]);
        let kept_o = t.keep(o);
        assert_eq!(probes_of(&mut t, o, int(8)), 2);
        let n = probes_of(&mut t, o, int(8));
        assert!(n == 1 || (stress && n == 2), "{n} probes");
        t.release(kept_o);
        t.release(kept_d);
        t.release_all(kept);
    });
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut iso = isolate(c);
        iso.with_cx_checkpoint(|cx| cx.run_script(wide_site(), "wide.js").map(|_| ()))
            .unwrap();
        let mut t = T { iso };
        let op = object_prototype(t.rt());
        let objs = megamorphic(&mut t, op, "x");
        let kept = t.keep_all(&objs);
        let d = dictionary_with_x(&mut t, op);
        let kept_d = t.keep(d);
        for _ in 0..3 {
            assert_eq!(probes_of(&mut t, d, int(7)), 2);
        }
        let o = object_with(t.rt(), op, &[("w", int(0)), ("x", int(5))]);
        let kept_o = t.keep(o);
        assert_eq!(probes_of(&mut t, o, int(5)), 2);
        let n = probes_of(&mut t, o, int(5));
        assert!(n == 1 || (stress && n == 2), "{n} probes");
        t.release(kept_o);
        t.release(kept_d);
        t.release_all(kept);
    });
}

// ---- the scale test of the fast-body probe (M10 LEVER-12 Codex nm-r1 F2) ----------------------

/// The runs of the fast-body probe (`ic::mega_fast`), counted before it reads the name operand.
fn fast_tries() -> u32 {
    ic::FAST_PROBE_TRIES.with(|c| c.get())
}

/// Function `f0(a)`: `return a.x`, with `pad` feedback slots before the site's slot (its index
/// is `pad`) and a pool of one constant, the name. So the load is a `Wide` (pad >= 256) or
/// `ExtraWide` (pad >= 65536) GetNamedProperty that only its slot operand widens: the byte that
/// a one-byte read of the name operand would take (byte 2, the register operand's low byte,
/// 0xfe for `a`) is no index in the pool. The unit and the prefix byte.
fn slot_widened_site(pad: u32) -> (ep9_bytecode::VerifiedUnit, u8) {
    let mut s = Asm::script(0);
    let k = s.b.function_constant(ep9_bytecode::FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.sta_global("f0");
    s.b.lda_undefined().return_();
    let mut b = ep9_bytecode::FunctionBuilder::new(meta("f0", 2, 0, 0, 0));
    let a = b.parameter(0);
    for _ in 0..pad {
        b.feedback_slot(ep9_bytecode::FeedbackSlotKind::LoadNamed);
    }
    let slot = b.feedback_slot(ep9_bytecode::FeedbackSlotKind::LoadNamed);
    let k = b.string_constant("x");
    assert_eq!(k.index(), 0);
    b.get_named_property(a, k, slot).return_();
    let u = unit("", vec![s.finish(), b.finish().unwrap()]);
    let code = &u.functions[1].bytecode;
    assert_eq!(code[1], ep9_bytecode::Opcode::GetNamedProperty as u8);
    assert_eq!(u.functions[1].constants.len(), 1);
    let prefix = code[0];
    (u, prefix)
}

/// Codex nm-r1 F2: a `Wide` and an `ExtraWide` GetNamedProperty that only the slot operand
/// widens, with a pool of one constant, at a megamorphic site: the fast body never tries its
/// probe (the scale test of `ic::load_or_mega`: a one-byte read of the name operand there would
/// take the register byte 0xfe as a constant index, out of the pool). The cold body probes, so
/// the second load of a shape hits there. The values are right in every case.
#[test]
fn a_slot_widened_get_named_never_tries_the_fast_probe() {
    each(slot_widened_at::<256, { ep9_bytecode::Opcode::Wide as u8 }>);
    each(slot_widened_at::<65_536, { ep9_bytecode::Opcode::ExtraWide as u8 }>);
}

/// [`a_slot_widened_get_named_never_tries_the_fast_probe`] for [`slot_widened_site`] with
/// `PAD`, which must give the prefix `PREFIX`, under the preset `c`.
fn slot_widened_at<const PAD: u32, const PREFIX: u8>(c: crate::IsolateConfig) {
    let stress = c.heap.gc_stress != 0;
    let (u, got) = slot_widened_site(PAD);
    assert_eq!(got, PREFIX);
    let mut iso = isolate(c);
    iso.with_cx_checkpoint(|cx| cx.run_script(u, "slot-wide.js").map(|_| ()))
        .unwrap();
    let mut t = T { iso };
    let op = object_prototype(t.rt());
    let tries = fast_tries();
    let pads = ["a", "b", "c", "d", "e", "f"];
    let mut kept = vec![];
    let mut objs = vec![];
    for (i, p) in pads.iter().enumerate() {
        let o = object_with(t.rt(), op, &[(p, int(0)), ("x", int(10 + i as i32))]);
        kept.push(t.keep(o));
        objs.push(o);
    }
    for (i, &o) in objs.iter().enumerate().take(5) {
        assert_eq!(t.ok(0, o, UNDEFINED), int(10 + i as i32));
    }
    assert_eq!(ic::state(t.vector(0), PAD), State::Megamorphic);
    let o = objs[5];
    assert_eq!(t.ok(0, o, UNDEFINED), int(15));
    let (h, f) = (handler_hits(), fast_hits());
    assert_eq!(t.ok(0, o, UNDEFINED), int(15));
    if !stress {
        assert_eq!((handler_hits(), fast_hits()), (h + 1, f));
    }
    assert_eq!(fast_tries(), tries);
    t.release_all(kept);
}

// ---- no probe for a ShapeId 0 receiver (M10 LEVER-12 Codex nm-r2 F1) --------------------------

/// The stub cache probes (`stub_cache::PROBES`) and fast-probe runs ([`fast_tries`]) of one
/// load `f0(v)` of `t`: `Ok` with the value, or `Err` if it threw.
fn probes_and_tries(t: &mut T, v: u64) -> (u32, u32, Result<u64, ()>) {
    let (p, f) = (stub_cache::PROBES.with(|c| c.get()), fast_tries());
    let r = t.call(0, v, UNDEFINED).map_err(|_| ());
    (
        stub_cache::PROBES.with(|c| c.get()) - p,
        fast_tries() - f,
        r,
    )
}

/// Codex nm-r2 F1: a string, a symbol and a BigInt (ShapeId 0, H16 K11) at a warmed
/// megamorphic one-byte GetNamedProperty site make no stub cache probe at all: the fast body
/// tests the ShapeId before its probe (`ic::load_or_mega`), and the runtime call tests
/// `is_object` before its own (`ic::mega_load`). No entry has ShapeId 0, so a probe could not
/// hit. An object at the same site still probes (the control). GC does not probe, so the
/// counts hold under GC stress too.
#[test]
fn a_string_symbol_or_bigint_at_a_megamorphic_site_never_probes() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut t = T::new(c, &[Site::Get("x")]);
        let op = object_prototype(t.rt());
        let objs = megamorphic(&mut t, op, "x");
        let kept = t.keep_all(&objs);
        // Warm: an entry for a new shape, then a hit in the fast body (one probe, one try).
        let o = object_with(t.rt(), op, &[("new", int(0)), ("x", int(8))]);
        let kept_o = t.keep(o);
        assert_eq!(t.ok(0, o, UNDEFINED), int(8));
        let (n, f, r) = probes_and_tries(&mut t, o);
        assert_eq!(r, Ok(int(8)));
        assert!(n == 1 || (stress && n == 2), "{n} probes");
        assert_eq!(f, 1);
        for _ in 0..2 {
            let s = string::new_string(t.rt(), "abc");
            assert_eq!(
                probes_and_tries(&mut t, s),
                (0, 0, Ok(UNDEFINED)),
                "a string"
            );
            let sym = symbol::new_symbol(t.rt(), UNDEFINED);
            assert_eq!(
                probes_and_tries(&mut t, sym),
                (0, 0, Ok(UNDEFINED)),
                "a symbol"
            );
            // The generic load throws in v0 (no BigInt.prototype).
            let b = crate::bigint::alloc_literal(t.rt(), false, &[5]);
            assert_eq!(probes_and_tries(&mut t, b), (0, 0, Err(())), "a BigInt");
        }
        assert_eq!(t.state(0), State::Megamorphic);
        t.release(kept_o);
        t.release_all(kept);
    });
}

// ---- two names of one shape in one entry (M10 LEVER-12 Codex nm-r2 F2) ------------------------

/// Two names (atoms of `t`, rooted by the handles) whose load entries for ShapeId `id` are the
/// same entry (`stub_cache::index_of`). The index comes from the atom's address and the
/// ShapeId, so a search finds them in each isolate (about 114 names on average for 8192
/// entries). Each name the search makes stays rooted until the end of the search, so a freed
/// atom's address cannot come back as a second name with the same bits.
fn two_names_in_one_entry(t: &T, id: u32) -> ([String; 2], [GlobalHandle; 2]) {
    let mut seen: std::collections::HashMap<usize, (String, GlobalHandle)> = Default::default();
    for i in 0..100_000 {
        let n = format!("c{i}");
        let k = key(t.rt(), &n);
        let h = t.keep(k);
        let at = stub_cache::index_of(Op::Load, id, k);
        if let Some((n0, h0)) = seen.remove(&at) {
            for (_, (_, hx)) in seen.drain() {
                t.release(hx);
            }
            let (k0, k1) = (key(t.rt(), &n0), key(t.rt(), &n));
            assert_ne!(k0, k1);
            assert_eq!(stub_cache::index_of(Op::Load, id, k0), at);
            return ([n0, n], [h0, h]);
        }
        seen.insert(at, (n, h));
    }
    panic!("no two names in one entry");
}

/// Functions `f0(a)`: `return a.<n0>` and `f1(a)`: `return a.<n1>`, one-byte GetNamedProperty
/// sites.
fn two_name_sites(names: &[String; 2]) -> ep9_bytecode::VerifiedUnit {
    let mut s = Asm::script(0);
    for i in 0..2 {
        let k = s.b.function_constant(ep9_bytecode::FunctionId(i + 1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global(&format!("f{i}"));
    }
    s.b.lda_undefined().return_();
    let mut fns = vec![s.finish()];
    for (i, n) in names.iter().enumerate() {
        let mut b = ep9_bytecode::FunctionBuilder::new(meta(&format!("f{i}"), 2, 0, 0, 0));
        let a = b.parameter(0);
        let slot = b.feedback_slot(ep9_bytecode::FeedbackSlotKind::LoadNamed);
        let k = b.string_constant(n.as_str());
        b.get_named_property(a, k, slot).return_();
        fns.push(b.finish().unwrap());
    }
    unit("", fns)
}

/// Codex nm-r2 F2: two names of one receiver shape whose entries are the same stub cache
/// entry. The entry holds the first name's case; a one-byte load of the second name at a
/// megamorphic site must miss the fast probe (`stub_cache::probe_at` compares the name) and
/// give its own value, never the first name's (both are fields of the prototype, so a hit
/// with the wrong entry reads the other field). Then the entry holds the second name.
#[test]
fn a_second_name_in_the_same_stub_cache_entry_misses_the_fast_probe() {
    each(|c| {
        let every = c.heap.gc_stress;
        let stress = every != 0;
        let mut t = T { iso: isolate(c) };
        let op = object_prototype(t.rt());
        let p = object_with(t.rt(), op, &[("p", int(0))]);
        let r = object_with(t.rt(), p, &[("own", int(0))]);
        let kept_r = t.keep(r);
        let rid = id(r);
        // The search makes about 114 atoms: no GC stress there; the loads below have it.
        t.rt().heap.set_gc_stress(0);
        let (names, kept_k) = two_names_in_one_entry(&t, rid);
        t.rt().heap.set_gc_stress(every);
        props::own_define(t.rt(), p, key(t.rt(), &names[0]), int(1), attr::DEFAULT);
        props::own_define(t.rt(), p, key(t.rt(), &names[1]), int(2), attr::DEFAULT);
        assert_eq!(id(r), rid);
        let u = two_name_sites(&names);
        t.iso
            .with_cx_checkpoint(|cx| cx.run_script(u, "two-names.js").map(|_| ()))
            .unwrap();
        // Both sites megamorphic (five other shapes, a missing name each).
        let pads = ["a", "b", "c", "d", "e"];
        let objs: [u64; 5] =
            core::array::from_fn(|i| object_with(t.rt(), op, &[(pads[i], int(0))]));
        let kept = t.keep_all(&objs);
        for i in 0..2 {
            for &o in &objs {
                assert_eq!(t.ok(i, o, UNDEFINED), UNDEFINED);
            }
            assert_eq!(ic::state(t.vector(i), SLOT), State::Megamorphic);
        }
        // The first name: its entry is filled, then a fast-body hit.
        assert_eq!(t.ok(0, r, UNDEFINED), int(1));
        let f = fast_hits();
        assert_eq!(t.ok(0, r, UNDEFINED), int(1));
        if !stress {
            assert_eq!(fast_hits(), f + 1);
            assert!(entry(&t, Op::Load, r, &names[0]).is_some());
            assert!(entry(&t, Op::Load, r, &names[1]).is_none());
        }
        // The second name, same shape, same entry: a fast-probe miss and its own value.
        let f = fast_hits();
        assert_eq!(t.ok(1, r, UNDEFINED), int(2));
        assert_eq!(fast_hits(), f);
        // Now the entry holds the second name: a hit for it, a miss for the first.
        let f = fast_hits();
        assert_eq!(t.ok(1, r, UNDEFINED), int(2));
        if !stress {
            assert_eq!(fast_hits(), f + 1);
        }
        let f = fast_hits();
        assert_eq!(t.ok(0, r, UNDEFINED), int(1));
        assert_eq!(fast_hits(), f);
        t.release_all(kept);
        t.release_all(kept_k.into());
        t.release(kept_r);
    });
}

// ---- two shapes of one name in one entry (M10 LEVER-12 Codex nm-r3) --------------------------

/// Two objects under `p` (rooted by the handles) with different shapes whose load entries for
/// `name` are the same entry (`stub_cache::index_of`), and with `name` at different slots.
/// Object pair `n` has the pads `s{n / 1024}`, `t{n / 32 % 32}` and `w{n % 32}` (0 each),
/// then `name` (slot 3) on side 0 and the pad `u` and `name` (slot 4) on side 1; `name` is
/// 100 + the object's number. The search adds at most 32 transitions to a shape, under the
/// cap (`shape::MAX_TRANSITIONS`, 64), so all the shapes are tree shapes. The search pairs a
/// side 0 object with a side 1 one only. The ShapeIds come one after the other, and the hash
/// spreads near ids well, so it needs some thousand objects (2,338 or 2,645 in each config on
/// mac a64). Each object the search makes stays rooted until the end of the search, so a
/// freed shape cannot come back with the same id.
fn two_shapes_in_one_entry(t: &T, p: u64, name: &str) -> ([u64; 2], [GlobalHandle; 2]) {
    let k = key(t.rt(), name);
    let mut seen: [std::collections::HashMap<usize, (u64, GlobalHandle)>; 2] = Default::default();
    for i in 0..2 * 32 * 32 * 32usize {
        let (side, n) = (i % 2, i / 2);
        let pads = [
            format!("s{}", n / 1024),
            format!("t{}", n / 32 % 32),
            format!("w{}", n % 32),
        ];
        let v = int(100 + i as i32);
        let mut props: Vec<(&str, u64)> = pads.iter().map(|q| (q.as_str(), int(0))).collect();
        if side == 1 {
            props.push(("u", int(0)));
        }
        props.push((name, v));
        let o = object_with(t.rt(), p, &props);
        assert!(!shape::is_dictionary(shape::shape_of(o)));
        let h = t.keep(o);
        let at = stub_cache::index_of(Op::Load, id(o), k);
        if let Some((o0, h0)) = seen[1 - side].remove(&at) {
            for m in &mut seen {
                for (_, (_, hx)) in m.drain() {
                    t.release(hx);
                }
            }
            assert_ne!(id(o0), id(o));
            assert_eq!(stub_cache::index_of(Op::Load, id(o0), k), at);
            return ([o0, o], [h0, h]);
        }
        // A second object of one side in one entry: the first one stays, the new one goes.
        if let Some((_, hx)) = seen[side].insert(at, (o, h)) {
            t.release(hx);
        }
    }
    panic!("no two shapes in one entry");
}

/// Codex nm-r3: two receiver shapes whose entries for one name are the same stub cache entry,
/// with the name at different slots. The entry holds the first shape's case; a one-byte load
/// of the name with the second receiver at a megamorphic site must miss the fast probe
/// (`stub_cache::probe_at` compares the ShapeId) and give its own value, never a field at the
/// first shape's offset (a pad, 0, or past the last field). Then the entry holds the second
/// shape.
#[test]
fn a_second_shape_in_the_same_stub_cache_entry_misses_the_fast_probe() {
    each(|c| {
        let every = c.heap.gc_stress;
        let stress = every != 0;
        let mut t = T::new(c, &[Site::Get("x")]);
        let op = object_prototype(t.rt());
        // The search makes some thousand objects: no GC stress there; the loads below have it.
        t.rt().heap.set_gc_stress(0);
        let (rs, kept_r) = two_shapes_in_one_entry(&t, op, "x");
        t.rt().heap.set_gc_stress(every);
        let want = rs.map(|r| own(t.rt(), r, "x").unwrap());
        assert!(want != [int(0); 2] && want[0] != want[1]);
        // The site megamorphic (five other shapes).
        let objs = five(&t, op);
        let kept = t.keep_all(&objs);
        for &o in &objs {
            t.ok(0, o, UNDEFINED);
        }
        assert_eq!(t.state(0), State::Megamorphic);
        // The first shape: its entry is filled, then a fast-body hit.
        assert_eq!(t.ok(0, rs[0], UNDEFINED), want[0]);
        let f = fast_hits();
        assert_eq!(t.ok(0, rs[0], UNDEFINED), want[0]);
        if !stress {
            assert_eq!(fast_hits(), f + 1);
            assert!(entry(&t, Op::Load, rs[0], "x").is_some());
            assert!(entry(&t, Op::Load, rs[1], "x").is_none());
        }
        // The second shape, same name, same entry: a fast-probe miss and its own value.
        let f = fast_hits();
        assert_eq!(t.ok(0, rs[1], UNDEFINED), want[1]);
        assert_eq!(fast_hits(), f);
        // Now the entry holds the second shape: a hit for it, a miss for the first.
        let f = fast_hits();
        assert_eq!(t.ok(0, rs[1], UNDEFINED), want[1]);
        if !stress {
            assert_eq!(fast_hits(), f + 1);
        }
        let f = fast_hits();
        assert_eq!(t.ok(0, rs[0], UNDEFINED), want[0]);
        assert_eq!(fast_hits(), f);
        t.release_all(kept);
        t.release_all(kept_r.into());
    });
}

// ---- the probe in the SetNamedProperty and DefineNamedOwnProperty fast bodies --------------------
// (M10-P2 LEVER-12 NAMED-STORE-MEGA)

/// The number of stores that the probe in the fast bodies of `SetNamedProperty` and
/// `DefineNamedOwnProperty` answered (`ic::store_or_mega`; [`store_hits`] counts them too).
fn fast_store_hits() -> u32 {
    ic::FAST_STORE_HITS.with(|c| c.get())
}

/// The counts ([`store_hits`], [`fast_store_hits`]) now.
fn store_counts() -> (u32, u32) {
    (store_hits(), fast_store_hits())
}

/// Stores `v` into `o.<name>` at site `i` of `t`: a hit in the fast body only if the stub cache
/// held an entry for it (without GC stress; under GC stress a collection may empty the table
/// first). A fast hit counts once in each count: the cold body did not run (its hit would move
/// [`store_hits`] only). True if the fast body stored.
fn store_counted(t: &mut T, i: usize, op: Op, o: u64, name: &str, v: u64, stress: bool) -> bool {
    let held = entry(t, op, o, name).is_some();
    let (h, f) = store_counts();
    t.ok(i, o, v);
    let hit = fast_store_hits() == f + 1;
    assert!(stress || hit || fast_store_hits() == f);
    assert!(stress || held || !hit, "a fast hit with no entry");
    if hit {
        assert_eq!(store_hits(), h + 1, "a fast hit counts once");
    }
    hit
}

/// The stub cache operation of store site `i` in a `T` of [`STORE_SITES`].
fn store_op_of(i: usize) -> Op {
    if i == 1 { Op::Define } else { Op::Store }
}

/// A sloppy 0x24, a 0x25 and a strict 0x24 site, all for `x`.
const STORE_SITES: [Site; 3] = [
    Site::Set("x", false),
    Site::Define("x"),
    Site::Set("x", true),
];

/// A one-byte 0x24 or 0x25 at a megamorphic site whose stub cache entry is there stores
/// through the fast body: `FAST_STORE_HITS` moves by one and `HANDLER_STORE_HITS` by one
/// only (the cold body did not run). A field store in the object and an add with room in the
/// object, for each site; then 40 shapes at one site (entries that conflicts replace miss and
/// are filled again). The values and the shapes are right in every case.
#[test]
fn the_store_fast_bodies_answer_megamorphic_stores_from_the_stub_cache() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut t = T::new(c, &STORE_SITES);
        let op = object_prototype(t.rt());
        let mut kept = Vec::new();
        for i in 0..3 {
            let objs = megamorphic_store(&mut t, i, op, "x");
            kept.push(t.keep_all(&objs));
        }
        for i in 0..3 {
            // A field store: own `x` in the object. One pad per site, so each site makes its
            // own entry (sites 0 and 2 share the `Store` entries).
            let pad = format!("f{i}");
            let f = |t: &T| object_with(t.rt(), op, &[(pad.as_str(), int(0)), ("x", int(3))]);
            let (o, o2) = (f(&t), f(&t));
            let (h, n) = store_counts();
            t.ok(i, o, int(7));
            assert_eq!(store_counts(), (h, n), "the first store fills the entry");
            assert!(stress || entry(&t, store_op_of(i), o, "x").is_some());
            let (h, n) = store_counts();
            t.ok(i, o2, int(8));
            assert_eq!(
                (own(t.rt(), o, "x"), own(t.rt(), o2, "x")),
                (Some(int(7)), Some(int(8)))
            );
            if !stress {
                assert_eq!(store_counts(), (h + 1, n + 1), "site {i}: a fast-body hit");
            }
            // An add with room in the object: the shape of the case's target.
            let pad = format!("g{i}");
            let a = |t: &T| object_with(t.rt(), op, &[(pad.as_str(), int(0))]);
            let (o, o2) = (a(&t), a(&t));
            t.ok(i, o, int(5));
            let (h, n) = store_counts();
            t.ok(i, o2, int(6));
            assert_eq!(
                (own(t.rt(), o, "x"), own(t.rt(), o2, "x")),
                (Some(int(5)), Some(int(6)))
            );
            assert_eq!(id(o), id(o2), "both took the add");
            if !stress {
                assert_eq!(store_counts(), (h + 1, n + 1), "site {i}: a fast-body add");
            }
        }
        // 40 shapes at site 0: each first store fills its entry, each second one is a fast
        // hit; then again in the other order.
        let (objs, kept_s) = shapes(&t, op, "x", 40);
        for (j, &o) in objs.iter().enumerate() {
            t.ok(0, o, int(j as i32));
            assert!(
                store_counted(&mut t, 0, Op::Store, o, "x", int(1000 + j as i32), stress) || stress
            );
            assert_eq!(own(t.rt(), o, "x"), Some(int(1000 + j as i32)));
        }
        for (j, &o) in objs.iter().enumerate().rev() {
            store_counted(&mut t, 0, Op::Store, o, "x", int(2000 + j as i32), stress);
            assert_eq!(own(t.rt(), o, "x"), Some(int(2000 + j as i32)));
        }
        assert_eq!(t.state(0), State::Megamorphic);
        t.release_all(kept_s);
        for k in kept {
            t.release_all(k);
        }
    });
}

/// A miss of the fast-body probe: the cold body and the runtime call run as before, and the
/// runtime call fills the entry, so the next store of the shape is a fast-body hit. A GC
/// empties the table: a miss again, then a fill. Not an object (a string, a symbol, a
/// BigInt: ShapeId 0, and an int32): no stub cache probe at all, and the result of the
/// generic store (a sloppy store to a primitive is ignored; a strict one throws).
#[test]
fn a_fast_store_probe_miss_takes_the_runtime_call_which_fills_the_entry() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut t = T::new(c, &STORE_SITES);
        let op = object_prototype(t.rt());
        let mut kept = Vec::new();
        for i in 0..3 {
            let objs = megamorphic_store(&mut t, i, op, "x");
            kept.push(t.keep_all(&objs));
        }
        for i in 0..3 {
            let pad = format!("n{i}");
            let o = object_with(t.rt(), op, &[(pad.as_str(), int(0)), ("x", int(1))]);
            let ko = t.keep(o);
            let sop = store_op_of(i);
            assert!(entry(&t, sop, o, "x").is_none());
            assert!(!store_counted(&mut t, i, sop, o, "x", int(2), stress));
            assert!(
                stress || entry(&t, sop, o, "x").is_some(),
                "the runtime call filled it"
            );
            assert!(store_counted(&mut t, i, sop, o, "x", int(3), stress) || stress);
            t.collect();
            assert!(entry(&t, sop, o, "x").is_none());
            assert!(!store_counted(&mut t, i, sop, o, "x", int(4), stress));
            assert!(store_counted(&mut t, i, sop, o, "x", int(5), stress) || stress);
            assert_eq!(own(t.rt(), o, "x"), Some(int(5)));
            t.release(ko);
        }
        let probes = |t: &mut T, i: usize, v: u64| {
            let p = stub_cache::PROBES.with(|c| c.get());
            let f = fast_store_hits();
            let r = t.call(i, v, int(1)).map(|_| ()).map_err(|_| ());
            assert_eq!(fast_store_hits(), f);
            (stub_cache::PROBES.with(|c| c.get()) - p, r)
        };
        for _ in 0..2 {
            let s = string::new_string(t.rt(), "abc");
            assert_eq!(probes(&mut t, 0, s), (0, Ok(())), "a string, sloppy");
            assert_eq!(probes(&mut t, 2, s).0, 0, "a string, strict");
            let sym = symbol::new_symbol(t.rt(), UNDEFINED);
            assert_eq!(probes(&mut t, 0, sym), (0, Ok(())), "a symbol, sloppy");
            let b = crate::bigint::alloc_literal(t.rt(), false, &[5]);
            assert_eq!(probes(&mut t, 0, b).0, 0, "a BigInt");
            assert_eq!(probes(&mut t, 0, int(7)), (0, Ok(())), "an int32, sloppy");
            assert_eq!(probes(&mut t, 2, int(7)), (0, Err(())), "an int32, strict");
        }
        assert_eq!(
            [t.state(0), t.state(1), t.state(2)],
            [State::Megamorphic; 3]
        );
        for k in kept {
            t.release_all(k);
        }
    });
}

/// The validity cell check of the fast-body probe: 0x24 (`store_with::<true>`) checks it for
/// an add case, 0x25 (`store_with::<false>`) does not, as the cold-body probe
/// (`ic::handler_mega_store`). A read-only `x` that the prototype gets after the entries were
/// made: the 0x24 add finds its entry, misses (the epoch), and the generic store adds
/// nothing; the 0x25 add still hits in the fast body and makes an own `x` (CreateDataProperty
/// does not read the chain). The slow path gives the same results.
#[test]
fn the_fast_store_probe_checks_the_validity_cell_for_set_and_not_for_define() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut t = T::new(
            c,
            &[
                Site::Set("x", false),
                Site::Define("x"),
                Site::Set("x", true),
            ],
        );
        let op = object_prototype(t.rt());
        let p = object_with(t.rt(), op, &[("pp", int(0))]);
        let kept_p = t.keep(p);
        let mut kept = Vec::new();
        for i in 0..3 {
            let objs = megamorphic_store(&mut t, i, p, "q");
            kept.push(t.keep_all(&objs));
        }
        let a = |t: &T| object_with(t.rt(), p, &[("lone", int(0))]);
        for i in [0, 1] {
            let (o, o2) = (a(&t), a(&t));
            t.ok(i, o, int(1));
            assert!(store_counted(&mut t, i, store_op_of(i), o2, "x", int(2), stress) || stress);
            assert_eq!(own(t.rt(), o2, "x"), Some(int(2)));
        }
        props::own_define(
            t.rt(),
            p,
            key(t.rt(), "x"),
            int(5),
            attr::ENUMERABLE | attr::CONFIGURABLE,
        );
        // 0x24, sloppy: the entry is there, but its validity cell is not valid.
        let o3 = a(&t);
        let ko3 = t.keep(o3);
        assert!(stress || entry(&t, Op::Store, o3, "x").is_some());
        let (h, f) = store_counts();
        t.ok(0, o3, int(9));
        assert_eq!(
            own(t.rt(), o3, "x"),
            None,
            "the read-only `x` blocks the add"
        );
        assert_eq!(store_counts(), (h, f));
        // 0x24, strict: a TypeError, and nothing added.
        t.throws(2, o3, int(9), "TypeError");
        assert_eq!(own(t.rt(), o3, "x"), None);
        // 0x25: no validity cell check; an own `x`, as the slow path makes.
        let o4 = a(&t);
        assert!(store_counted(&mut t, 1, Op::Define, o4, "x", int(9), stress) || stress);
        assert_eq!(own(t.rt(), o4, "x"), Some(int(9)));
        assert_eq!(own(t.rt(), p, "x"), Some(int(5)));
        t.release(ko3);
        for k in kept {
            t.release_all(k);
        }
        t.release(kept_p);
    });
}

/// An add whose entry has no room in the SLOTS cell misses the fast-body probe (the growth
/// allocates), and the cold-body probe misses too; the runtime call grows the SLOTS cell
/// through the entry (`ic::mega_store`, `ic::grow_add`).
#[test]
fn a_megamorphic_add_with_no_room_misses_the_fast_store_probe() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut t = T::new(c, &[Site::Set("x", false), Site::Define("x")]);
        let op = object_prototype(t.rt());
        let mut kept = Vec::new();
        for i in [0, 1] {
            let objs = megamorphic_store(&mut t, i, op, "x");
            kept.push(t.keep_all(&objs));
        }
        for i in [0, 1] {
            let first = at_capacity(t.rt(), op);
            t.ok(i, first, int(1));
            let q = at_capacity(t.rt(), op);
            assert_eq!(id(q), {
                let r = at_capacity(t.rt(), op);
                id(r)
            });
            let (counts, grown) = (store_counts(), grow_adds());
            t.ok(i, q, int(2));
            assert_eq!(own(t.rt(), q, "x"), Some(int(2)));
            assert_eq!(id(q), id(first));
            assert_eq!(store_counts(), counts, "no probe hit can grow");
            if !stress {
                assert_eq!(grow_adds(), grown + 1, "the runtime call grew it");
            }
        }
        for k in kept {
            t.release_all(k);
        }
    });
}

/// A read-only field, a frozen object, a non-extensible object and a dictionary-mode object
/// at megamorphic 0x24 (sloppy and strict) and 0x25 sites: no stub cache entry is made for
/// them, so the fast-body probe never stores; the result is that of the generic store (a
/// sloppy store that fails is ignored; a strict one and a CreateDataPropertyOrThrow throw a
/// TypeError).
#[test]
fn read_only_frozen_non_extensible_and_dictionary_receivers_take_the_generic_store() {
    each(|c| {
        let mut t = T::new(c, &STORE_SITES);
        let op = object_prototype(t.rt());
        let mut kept = Vec::new();
        for i in 0..3 {
            let objs = megamorphic_store(&mut t, i, op, "x");
            kept.push(t.keep_all(&objs));
        }
        let x = key(t.rt(), "x");
        let f = fast_store_hits();
        // A read-only, non-configurable `x` in a fast shape.
        let ro = object_with(t.rt(), op, &[("ro", int(0))]);
        props::own_define(t.rt(), ro, x, int(3), attr::ENUMERABLE);
        assert!(!shape::is_dictionary(shape::shape_of(ro)));
        // Frozen: every property read-only and non-configurable, then non-extensible.
        let fz = object_with(t.rt(), op, &[("fz", int(0))]);
        props::own_define(t.rt(), fz, key(t.rt(), "fz"), int(0), attr::ENUMERABLE);
        props::own_define(t.rt(), fz, x, int(3), attr::ENUMERABLE);
        props::prevent_extensions(t.rt(), fz);
        let kept_r = t.keep_all(&[ro, fz]);
        for _ in 0..3 {
            for o in [ro, fz] {
                t.ok(0, o, int(9));
                t.throws(2, o, int(9), "TypeError");
                t.throws(1, o, int(9), "TypeError");
                assert_eq!(own(t.rt(), o, "x"), Some(int(3)));
            }
        }
        // Non-extensible: a field store writes, an add fails.
        let ne = object_with(t.rt(), op, &[("ne", int(0)), ("x", int(1))]);
        props::prevent_extensions(t.rt(), ne);
        let na = object_with(t.rt(), op, &[("na", int(0))]);
        props::prevent_extensions(t.rt(), na);
        let kept_n = t.keep_all(&[ne, na]);
        for j in 0..3 {
            for i in 0..3 {
                t.ok(i, ne, int(10 * j + i as i32));
                assert_eq!(own(t.rt(), ne, "x"), Some(int(10 * j + i as i32)));
            }
            t.ok(0, na, int(9));
            t.throws(2, na, int(9), "TypeError");
            t.throws(1, na, int(9), "TypeError");
            assert_eq!(own(t.rt(), na, "x"), None);
        }
        // Dictionary mode (a delete that is not the last, OM-10): a field store and an add.
        let d = dictionary_with_x(&mut t, op);
        let da = object_with(t.rt(), op, &[("pad", int(0)), ("y", int(0))]);
        let pad = key(t.rt(), "pad");
        assert!(property::delete(&mut t.iso.cx, da, pad, true).unwrap());
        assert!(shape::is_dictionary(shape::shape_of(da)));
        let kept_d = t.keep_all(&[d, da]);
        for i in 0..3 {
            t.ok(i, d, int(20 + i as i32));
            assert_eq!(own(t.rt(), d, "x"), Some(int(20 + i as i32)));
            assert!(entry(&t, store_op_of(i), d, "x").is_none());
        }
        t.ok(1, da, int(30));
        assert_eq!(own(t.rt(), da, "x"), Some(int(30)));
        assert_eq!(fast_store_hits(), f, "no fast-body store");
        t.release_all(kept_d);
        t.release_all(kept_n);
        t.release_all(kept_r);
        for k in kept {
            t.release_all(k);
        }
    });
}

/// A proxy at megamorphic 0x24 and 0x25 sites: the fast-body probe never stores (no entry is
/// made for a proxy), and the store goes to the target through the proxy's internal methods.
#[test]
fn a_proxy_at_a_megamorphic_store_site_takes_the_generic_store() {
    each(|c| {
        let mut t = T::new(c, &STORE_SITES);
        let op = object_prototype(t.rt());
        let mut kept = Vec::new();
        for i in 0..3 {
            let objs = megamorphic_store(&mut t, i, op, "x");
            kept.push(t.keep_all(&objs));
        }
        let target = object_with(t.rt(), op, &[("t", int(0))]);
        let handler = object_with(t.rt(), op, &[]);
        let kept_t = t.keep_all(&[target, handler]);
        let p = crate::proxy::proxy_create(&mut t.iso.cx, target, handler).unwrap();
        let kept_p = t.keep(p);
        let f = fast_store_hits();
        for j in 0..3 {
            for i in 0..3 {
                let v = int(10 * j + i as i32);
                t.ok(i, p, v);
                assert_eq!(own(t.rt(), target, "x"), Some(v));
            }
        }
        assert_eq!(fast_store_hits(), f);
        t.release(kept_p);
        t.release_all(kept_t);
        for k in kept {
            t.release_all(k);
        }
    });
}

/// The stub cache probes (`stub_cache::PROBES`) of one store `f<i>(o, v)` of `t`.
fn store_probes(t: &mut T, i: usize, o: u64, v: u64) -> u32 {
    let p = stub_cache::PROBES.with(|c| c.get());
    t.ok(i, o, v);
    stub_cache::PROBES.with(|c| c.get()) - p
}

/// The probes of one megamorphic store (`ic-stats` counts a hit where it stores and a miss
/// once, in the runtime call, `ic::mega_store`): a hit in the fast body probes once. A miss of
/// a one-byte 0x24 or 0x25 probes three times today: in the fast body (`ic::store_or_mega`),
/// in the cold body (`ic::handler_mega_store`) and in the runtime call. The load form probes
/// twice (`ic::get_named_cold_at` skips the cold-body probe); the store form needs a
/// `slow/property.rs` change for that (M10-P2 LEVER-12 probe notes, open item 2). GC does not
/// probe, so the miss counts hold under GC stress too.
#[test]
fn a_megamorphic_store_probes_once_on_a_fast_hit_and_three_times_on_a_miss() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut t = T::new(c, &STORE_SITES);
        let op = object_prototype(t.rt());
        let mut kept = Vec::new();
        for i in 0..3 {
            let objs = megamorphic_store(&mut t, i, op, "x");
            kept.push(t.keep_all(&objs));
        }
        let d = dictionary_with_x(&mut t, op);
        let kept_d = t.keep(d);
        for i in 0..3 {
            for _ in 0..2 {
                assert_eq!(store_probes(&mut t, i, d, int(1)), 3, "site {i}, a miss");
            }
            let pad = format!("h{i}");
            let o = object_with(t.rt(), op, &[(pad.as_str(), int(0)), ("x", int(8))]);
            let ko = t.keep(o);
            assert_eq!(store_probes(&mut t, i, o, int(2)), 3, "site {i}, the fill");
            let n = store_probes(&mut t, i, o, int(3));
            assert!(n == 1 || (stress && n == 3), "site {i}: {n} probes");
            assert_eq!(own(t.rt(), o, "x"), Some(int(3)));
            t.release(ko);
        }
        t.release(kept_d);
        for k in kept {
            t.release_all(k);
        }
    });
}

/// Function `f0(a, b)`: `a.x = b` as a `Wide` SetNamedProperty: the name constant at index
/// 300 (`name_wide`), or the slot at index `pad` >= 256 with a pool of one constant (a byte
/// that a one-byte read of the name operand would take is no index in the pool). The unit
/// and the prefix byte.
fn wide_store_site(name_wide: bool, pad: u32) -> (ep9_bytecode::VerifiedUnit, u8) {
    let mut s = Asm::script(0);
    let k = s.b.function_constant(ep9_bytecode::FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.sta_global("f0");
    s.b.lda_undefined().return_();
    let mut b = ep9_bytecode::FunctionBuilder::new(meta("f0", 2, 0, 0, 0));
    let (a, x) = (b.parameter(0), b.parameter(1));
    for _ in 0..pad {
        b.feedback_slot(ep9_bytecode::FeedbackSlotKind::StoreNamed);
    }
    let slot = b.feedback_slot(ep9_bytecode::FeedbackSlotKind::StoreNamed);
    if name_wide {
        for i in 0..300 {
            b.number_constant(f64::from(i) + 0.5);
        }
    }
    let k = b.string_constant("x");
    assert_eq!(k.index() >= 256, name_wide);
    b.ldar(x).set_named_property(a, k, slot);
    b.return_();
    let u = unit("", vec![s.finish(), b.finish().unwrap()]);
    let code = &u.functions[1].bytecode;
    let at = code
        .iter()
        .position(|&c| c == ep9_bytecode::Opcode::SetNamedProperty as u8)
        .unwrap();
    let prefix = code[at - 1];
    (u, prefix)
}

/// A `Wide` and an `ExtraWide` SetNamedProperty at a megamorphic site (the name or only the
/// slot widened): the fast body runs `ic::store` (the scale test of `ic::store_or_mega`), so
/// the second store of a shape hits in the cold body and never in the fast body.
#[test]
fn a_prefixed_set_named_probes_in_its_cold_body_only() {
    each(prefixed_store_at::<true, 0, { ep9_bytecode::Opcode::Wide as u8 }>);
    each(prefixed_store_at::<false, 256, { ep9_bytecode::Opcode::Wide as u8 }>);
    each(prefixed_store_at::<false, 65_536, { ep9_bytecode::Opcode::ExtraWide as u8 }>);
}

/// [`a_prefixed_set_named_probes_in_its_cold_body_only`] for [`wide_store_site`] with
/// `NAME_WIDE` and `PAD`, which must give the prefix `PREFIX`, under the preset `c`.
fn prefixed_store_at<const NAME_WIDE: bool, const PAD: u32, const PREFIX: u8>(
    c: crate::IsolateConfig,
) {
    let stress = c.heap.gc_stress != 0;
    let (u, got) = wide_store_site(NAME_WIDE, PAD);
    assert_eq!(got, PREFIX);
    let mut iso = isolate(c);
    iso.with_cx_checkpoint(|cx| cx.run_script(u, "wide-store.js").map(|_| ()))
        .unwrap();
    let mut t = T { iso };
    let op = object_prototype(t.rt());
    let pads = ["a", "b", "c", "d", "e", "f"];
    let mut kept = vec![];
    let mut objs = vec![];
    for (i, p) in pads.iter().enumerate() {
        let o = object_with(t.rt(), op, &[(p, int(0)), ("x", int(10 + i as i32))]);
        kept.push(t.keep(o));
        objs.push(o);
    }
    for &o in objs.iter().take(5) {
        t.ok(0, o, int(1));
    }
    assert_eq!(ic::state(t.vector(0), PAD), State::Megamorphic);
    let o = objs[5];
    t.ok(0, o, int(2));
    let (h, f) = store_counts();
    t.ok(0, o, int(3));
    assert_eq!(own(t.rt(), o, "x"), Some(int(3)));
    if !stress {
        assert_eq!(store_counts(), (h + 1, f), "a cold-body hit");
    }
    assert_eq!(fast_store_hits(), f);
    t.release_all(kept);
}

/// The SLOTS cell of `o` (undefined if it has none) and its capacity (0 then).
fn slots_of(o: u64) -> (u64, i32) {
    match object::rd(object::cell(o), object::obj::SLOTS) {
        UNDEFINED => (UNDEFINED, 0),
        s => (s, object::rd_i32(object::cell(s), shape::slf::CAPACITY)),
    }
}

/// A fast-body hit out of the object (the SLOTS branch of `ic::store_with`): a field store and
/// an add with room, at each of the 0x24, 0x25 and strict 0x24 sites. The objects and their
/// SLOTS cells are old (a major GC); the stored values are young strings. A minor GC then
/// runs V1 (the remembered-set verifier, `verify_after_gc`) first, so a store with no write
/// barrier fails there. The values are right after the minor GC and after a major GC.
/// (M10-P2 LEVER-12, Codex lv12-r1 F2: the other fast-hit tests store in the object only.)
#[test]
fn a_fast_store_hit_out_of_the_object_keeps_a_young_value_over_a_minor_gc() {
    each(|c| {
        let stress = c.heap.gc_stress != 0;
        let mut t = T::new(c, &STORE_SITES);
        let op = object_prototype(t.rt());
        let mut kept = Vec::new();
        for i in 0..3 {
            let objs = megamorphic_store(&mut t, i, op, "x");
            kept.push(t.keep_all(&objs));
        }
        for i in 0..3 {
            let s0 = string::new_string(t.rt(), &format!("old{i}"));
            // A field store: own `x` out of the object (after `h<i>`, the first field there).
            let h = key(t.rt(), &format!("h{i}"));
            let f = |t: &T| {
                let o = at_capacity(t.rt(), op);
                props::own_define(t.rt(), o, h, int(0), attr::DEFAULT);
                props::own_define(t.rt(), o, key(t.rt(), "x"), s0, attr::DEFAULT);
                o
            };
            let (o, o2) = (f(&t), f(&t));
            assert_ne!(slots_of(o2).0, UNDEFINED, "x is out of the object");
            assert_eq!(id(o), id(o2));
            // An add with room: the SLOTS cell holds `g<i>` and has room for `x`.
            let g = key(t.rt(), &format!("g{i}"));
            let a = |t: &T| {
                let o = at_capacity(t.rt(), op);
                props::own_define(t.rt(), o, g, int(0), attr::DEFAULT);
                o
            };
            let (a1, a2) = (a(&t), a(&t));
            assert!(slots_of(a2).1 > 1, "the SLOTS cell has room for x");
            assert_eq!(id(a1), id(a2));
            let ks = t.keep_all(&[s0, o, o2, a1, a2]);
            // A major GC: the objects, their SLOTS cells and `s0` are old. It also empties the
            // stub cache, so the next stores fill the entries again.
            t.collect();
            let sop = store_op_of(i);
            t.ok(i, o, s0);
            t.ok(i, a1, s0);
            assert!(
                stress || entry(&t, sop, o2, "x").is_some(),
                "site {i}: the field entry"
            );
            assert!(
                stress || entry(&t, sop, a2, "x").is_some(),
                "site {i}: the add entry"
            );
            let y = string::new_string(t.rt(), &format!("young{i}"));
            let z = string::new_string(t.rt(), &format!("added{i}"));
            let (hs, n) = store_counts();
            t.ok(i, o2, y);
            if !stress {
                assert_eq!(
                    store_counts(),
                    (hs + 1, n + 1),
                    "site {i}: a fast-body field hit"
                );
            }
            let (hs, n) = store_counts();
            t.ok(i, a2, z);
            if !stress {
                assert_eq!(store_counts(), (hs + 1, n + 1), "site {i}: a fast-body add");
            }
            assert_eq!(id(a2), id(a1), "both took the add");
            t.rt().heap.collect_minor();
            let text = |t: &T, o: u64| own(t.rt(), o, "x").map(string::to_rust_string);
            assert_eq!(text(&t, o2).as_deref(), Some(format!("young{i}").as_str()));
            assert_eq!(text(&t, a2).as_deref(), Some(format!("added{i}").as_str()));
            t.collect();
            assert_eq!(text(&t, o2).as_deref(), Some(format!("young{i}").as_str()));
            assert_eq!(text(&t, a2).as_deref(), Some(format!("added{i}").as_str()));
            t.release_all(ks);
        }
        for k in kept {
            t.release_all(k);
        }
    });
}
