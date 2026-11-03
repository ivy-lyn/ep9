//! Tests of Dictionary v1 (`props_index.rs`, PROPS_INDEXED 84) (owner M6-P1 DICT1; the H17
//! "Dictionary v1" part): the move from PROPS above 8 entries, find, insert and delete in the
//! indexed form (a hole, no tail shift), compaction before an enumeration, re-index on grow,
//! the form below 9 entries (no index), content probes and symbols, the GC trace of kind 84,
//! a compaction move in an old large table, and 1,021 properties deleted in random order.
//! Every test runs with the heap verifier after each collection, most of them also under GC
//! stress 1. Values live in Rust locals (the conservative stack scan keeps them), in a JS array
//! or in the object under test; no Rust collection holds a Value (rule 8).

use std::hint::black_box;

use ep9_heap::CellPtr;

use super::{config, isolate, on_big_stack};
use crate::frame::{UNDEFINED, int32_bits, int32_of};
use crate::interp::Cx as VmCx;
use crate::isolate::IsolateConfig;
use crate::key::Probe;
use crate::object::{self, attr, obj};
use crate::props_index::{self, pif};
use crate::runtime::{Root, Runtime, val};
use crate::shape::{self, MAX_FAST_PROPERTIES};
use crate::{Dispatch, atoms, kinds, property, props, string, symbol};

// ---- helpers ------------------------------------------------------------------------------------

/// GC stress 1 when `stress`, the heap verifier after every collection either way.
fn verified(stress: bool) -> IsolateConfig {
    let mut c = config(Dispatch::DEFAULT, stress);
    c.heap.verify_after_gc = true;
    c
}

/// Runs `f` with a new isolate, without and with GC stress, on a big stack; collects and
/// verifies the heap after `f`.
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

fn put(rt: &Runtime, o: u64, name: &str, v: u64) {
    props::own_define(rt, o, k(rt, name), v, attr::DEFAULT);
}

fn own(rt: &Runtime, o: u64, name: &str) -> Option<u64> {
    props::own_get(o, k(rt, name)).map(|(v, _)| v)
}

fn del(rt: &Runtime, o: u64, name: &str) -> bool {
    props::own_delete(rt, o, k(rt, name)).is_some()
}

/// The table of the dictionary-mode object `o`.
fn table(o: u64) -> CellPtr {
    assert!(shape::is_dictionary(shape::shape_of(o)), "not a dictionary");
    object::cell(object::rd(object::cell(o), obj::SLOTS))
}

/// The kind of the table of `o`.
fn table_kind(o: u64) -> u8 {
    kinds::cell_kind(object::rd(object::cell(o), obj::SLOTS))
}

/// (capacity, live count, used, buckets or 0) of the PROPS_INDEXED table of `o`.
fn state(o: u64) -> (usize, usize, usize, usize) {
    assert_eq!(table_kind(o), kinds::PROPS_INDEXED.get());
    props_index::state(table(o))
}

/// The own keys of `o` as texts, from `own_count` and `own_entry` (no allocation).
fn keys(o: u64) -> Vec<String> {
    (0..props::own_count(o))
        .map(|i| string::str_view(props::own_entry(o, i).0).to_string_lossy())
        .collect()
}

/// `[[OwnPropertyKeys]]` of `o` as texts.
fn own_keys(cx: &mut VmCx, o: u64) -> Vec<String> {
    let a = property::own_property_keys(cx, o);
    (0..object::array_len(a))
        .map(|i| string::str_view(object::array_get(a, i).unwrap()).to_string_lossy())
        .collect()
}

fn names(prefix: &str, ids: impl IntoIterator<Item = usize>) -> Vec<String> {
    ids.into_iter().map(|i| format!("{prefix}{i}")).collect()
}

/// A dictionary-mode object with the properties `p0 .. p{n-1}` = their numbers, added in that
/// order after the object went to dictionary mode with no property.
fn dict_with(rt: &Runtime, n: usize) -> u64 {
    let o = object::new_plain_object(rt);
    props::to_dictionary(rt, o);
    for i in 0..n {
        put(rt, o, &format!("p{i}"), int(i as i32));
    }
    o
}

/// A fixed-seed xorshift64 (the delete order is the same on every run and host).
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

// ---- the form ---------------------------------------------------------------------------------

/// A dictionary keeps a PROPS table up to 8 entries; the 9th moves it to PROPS_INDEXED with
/// an index of twice its capacity, and every key is found through the index.
#[test]
fn the_ninth_entry_moves_a_props_table_to_props_indexed() {
    each(|cx, rt| {
        let o = dict_with(rt, 8);
        assert_eq!(table_kind(o), kinds::PROPS.get());
        put(rt, o, "p8", int(8));
        assert_eq!(state(o), (16, 9, 9, 32));
        for i in 0..9 {
            assert_eq!(own(rt, o, &format!("p{i}")), Some(int(i)));
        }
        assert_eq!(own(rt, o, "absent"), None);
        assert_eq!(keys(o), names("p", 0..9));
        assert_eq!(own_keys(cx, o), names("p", 0..9));
        // The layout of the export: entry 0 at +32, the live count at +16.
        let t = table(o);
        assert_eq!(object::rd(t, pif::ENTRIES), k(rt, "p0"));
        assert_eq!(object::rd(t, pif::ENTRIES + pif::ENTRY * 8 + 8), int(8));
        assert_eq!(int32_of(object::rd(t, pif::COUNT)), 9);
    });
}

/// OM-10 on a fast object: above 8 properties the table is PROPS_INDEXED at once, at 8 or
/// fewer a PROPS table, both in property order.
#[test]
fn to_dictionary_picks_the_form_by_the_property_count() {
    each(|cx, rt| {
        let big = object::new_plain_object(rt);
        for i in 0..12 {
            put(rt, big, &format!("b{i}"), int(i));
        }
        assert!(!shape::is_dictionary(shape::shape_of(big)));
        assert!(del(rt, big, "b4"), "a delete that is not the last (OM-10)");
        // 12 entries went in; b4 is a hole now.
        assert_eq!(state(big), (16, 11, 12, 32));
        let want: Vec<String> = names("b", (0..12).filter(|&i| i != 4));
        assert_eq!(own_keys(cx, big), want);
        let small = object::new_plain_object(rt);
        for i in 0..8 {
            put(rt, small, &format!("s{i}"), int(i));
        }
        assert!(del(rt, small, "s4"));
        assert_eq!(table_kind(small), kinds::PROPS.get());
        assert_eq!(keys(small), names("s", (0..8).filter(|&i| i != 4)));
    });
}

/// A delete in the indexed form leaves a hole (no tail shift): `used` stays, the live count
/// drops, other keys are still found and the deleted one is not. An enumeration compacts the
/// table first, so the keys come in insertion order, and the rebuilt index still finds them.
#[test]
fn a_delete_leaves_a_hole_and_an_enumeration_compacts() {
    each(|cx, rt| {
        let o = dict_with(rt, 20);
        assert_eq!(state(o), (32, 20, 20, 64));
        assert!(del(rt, o, "p3") && del(rt, o, "p10"));
        assert!(!del(rt, o, "p3"), "a second delete finds nothing");
        assert_eq!(state(o), (32, 18, 20, 64));
        assert_eq!((own(rt, o, "p3"), own(rt, o, "p10")), (None, None));
        for i in [0, 4, 9, 11, 19] {
            assert_eq!(own(rt, o, &format!("p{i}")), Some(int(i)));
        }
        // The hole's words are undefined: the GC keeps nothing of it.
        let t = table(o);
        for w in 0..3 {
            assert_eq!(object::rd(t, props_index::entry(3) + 8 * w), UNDEFINED);
        }
        let want = names("p", (0..20).filter(|&i| i != 3 && i != 10));
        assert_eq!(own_keys(cx, o), want);
        assert_eq!(state(o), (32, 18, 18, 64), "compacted by the enumeration");
        for i in (0..20).filter(|&i| i != 3 && i != 10) {
            assert_eq!(own(rt, o, &format!("p{i}")), Some(int(i)));
        }
        // A re-added key goes to the end (insertion order, §10.1.11.1).
        put(rt, o, "p3", int(-3));
        let mut want = want;
        want.push("p3".into());
        assert_eq!(keys(o), want);
        assert_eq!(own(rt, o, "p3"), Some(int(-3)));
    });
}

/// The table doubles when it is full and more than half live (a new table, a new index);
/// add/delete churn with few live entries compacts in place and keeps the capacity.
#[test]
fn growth_reindexes_and_churn_compacts_in_place() {
    each(|_, rt| {
        let o = dict_with(rt, 100);
        assert_eq!(state(o), (128, 100, 100, 256));
        for i in 0..100 {
            assert_eq!(own(rt, o, &format!("p{i}")), Some(int(i)));
        }
        // Churn: 40 live entries, then 600 rounds of add one, delete the oldest.
        for i in 0..60 {
            assert!(del(rt, o, &format!("p{i}")));
        }
        for i in 100..700 {
            put(rt, o, &format!("p{i}"), int(i));
            assert!(del(rt, o, &format!("p{}", i - 40)));
            let (cap, live, used, _) = state(o);
            assert_eq!(live, 40);
            assert!(cap == 128 && used <= cap, "{cap} {used}");
        }
        assert_eq!(keys(o), names("p", 660..700));
        for i in 660..700 {
            assert_eq!(own(rt, o, &format!("p{i}")), Some(int(i)));
        }
        assert_eq!(own(rt, o, "p659"), None);
    });
}

/// At 8 live entries the table drops its index (it stays kind 84: the move is one-way) and
/// scans; the 9th entry builds an index again.
#[test]
fn eight_live_entries_drop_the_index_and_the_ninth_builds_it_again() {
    each(|_, rt| {
        let o = dict_with(rt, 10);
        assert!(del(rt, o, "p0"));
        assert_eq!(state(o), (16, 9, 10, 32));
        assert!(del(rt, o, "p5"));
        assert_eq!(state(o), (16, 8, 8, 0), "compacted, no index");
        assert_eq!(keys(o), names("p", [1, 2, 3, 4, 6, 7, 8, 9]));
        assert_eq!(own(rt, o, "p9"), Some(int(9)));
        assert_eq!(own(rt, o, "p5"), None);
        // Without an index a delete shifts the tail (at most 8 entries), as in PROPS.
        assert!(del(rt, o, "p2"));
        assert_eq!(state(o), (16, 7, 7, 0));
        assert_eq!(keys(o), names("p", [1, 3, 4, 6, 7, 8, 9]));
        put(rt, o, "x", int(-1));
        put(rt, o, "y", int(-2));
        assert_eq!(state(o), (16, 9, 9, 32));
        assert_eq!(own(rt, o, "y"), Some(int(-2)));
        assert_eq!(own(rt, o, "p8"), Some(int(8)));
    });
}

/// A non-atom string probe (a content compare) and symbol keys find their entries through the
/// index; attribute changes and accessors keep their entry.
#[test]
fn content_probes_symbols_and_attributes_in_the_indexed_form() {
    each(|cx, rt| {
        let o = dict_with(rt, 12);
        let s1 = symbol::new_symbol(rt, UNDEFINED);
        props::own_define(rt, o, s1, int(100), attr::DEFAULT);
        let s2 = symbol::new_symbol(rt, UNDEFINED);
        props::own_define(rt, o, s2, int(200), attr::DEFAULT);
        assert_eq!(props::own_get(o, s1), Some((int(100), attr::DEFAULT)));
        assert_eq!(props::own_get(o, s2), Some((int(200), attr::DEFAULT)));
        let probe = string::new_string(rt, "p7");
        assert!(!string::is_atom(probe));
        assert_eq!(
            props::own_get_with(o, probe, Probe::Content),
            Some((int(7), attr::DEFAULT))
        );
        let missing = string::new_string(rt, "p77");
        assert_eq!(props::own_get_with(o, missing, Probe::Content), None);
        // Delete the entry before p7's, then probe by content past the hole.
        assert!(del(rt, o, "p6"));
        assert_eq!(
            props::own_get_with(o, probe, Probe::Content),
            Some((int(7), attr::DEFAULT))
        );
        // A new attribute set keeps the entry and its place.
        props::own_define(rt, o, k(rt, "p2"), int(22), attr::WRITABLE);
        assert_eq!(
            props::own_get(o, k(rt, "p2")),
            Some((int(22), attr::WRITABLE))
        );
        assert!(
            props::own_store_if_writable(rt, o, k(rt, "p2"), Probe::Bits, int(23))
                .is_some_and(|stored| stored)
        );
        assert_eq!(own(rt, o, "p2"), Some(int(23)));
        let want: Vec<String> = names("p", (0..12).filter(|&i| i != 6));
        assert_eq!(props::own_count(o), 13);
        for (i, name) in want.iter().enumerate() {
            assert_eq!(props::own_entry(o, i).0, k(rt, name));
        }
        assert_eq!(props::own_entry(o, 11).0, s1);
        assert_eq!(props::own_entry(o, 12).0, s2);
        // [[OwnPropertyKeys]]: the strings in order, then the symbols.
        let a = property::own_property_keys(cx, o);
        assert_eq!(object::array_len(a), 13);
        assert_eq!(object::array_get(a, 0), Some(k(rt, "p0")));
        assert_eq!(object::array_get(a, 11), Some(s1));
        assert_eq!(object::array_get(a, 12), Some(s2));
    });
}

// ---- the GC -----------------------------------------------------------------------------------

/// Masks a cell address so the test's frame never holds it (the conservative scan, H10).
const MASK: usize = 0x5a5a_5a5a_5a5a_5a5a;

/// Overwrites 256 KiB of stack below the caller (plan v10 rule 35), so stale words of returned
/// frames hold no cell for the conservative scan.
#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    black_box(&mut words);
}

/// Stores a new object as the value of the own property `name` of `o` (it has one) and
/// returns a weak handle to it and its masked address; out of line, so the object stays in no
/// frame of the caller.
#[inline(never)]
fn hold_new_object(rt: &Runtime, o: u64, name: &str) -> (ep9_heap::WeakHandle, usize) {
    let held = object::new_object(rt, rt.root(Root::ObjectPrototype));
    assert!(props::own_replace(rt, o, k(rt, name), held));
    let h = rt.heap.new_weak(val(held));
    (h, black_box(held as usize) ^ MASK)
}

/// The kind-84 trace keeps every live entry's value; a deleted entry's value dies. After a
/// scrub and a full GC whose conservative scan found neither object, the weak handle of the
/// live entry's value still reads it (and the object reads it back), and the deleted one's is
/// cleared (rule 35).
#[test]
fn the_props_indexed_trace_keeps_live_values_and_drops_deleted_ones() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut c = verified(stress);
            c.heap.record_conservative_roots = true;
            let iso = isolate(c);
            let rt = &*iso.rt;
            let o = dict_with(rt, 20);
            let root = rt.heap.new_global(val(o));
            let (live, live_addr) = hold_new_object(rt, o, "p5");
            let (dead, dead_addr) = hold_new_object(rt, o, "p7");
            assert!(del(rt, o, "p7"));
            scrub_stack();
            rt.heap.collect();
            let css = rt.heap.last_conservative_roots();
            for addr in [live_addr ^ MASK, dead_addr ^ MASK] {
                assert!(
                    !css.iter().any(|c| c.addr() == addr),
                    "the conservative scan kept a test object: the check proves nothing"
                );
            }
            let v = rt.heap.weak(live).expect("a live entry's value survives");
            assert_eq!(own(rt, o, "p5"), Some(v.to_bits()));
            assert!(rt.heap.weak(dead).is_none(), "a deleted entry's value dies");
            iso.verify_heap().unwrap();
            rt.heap.drop_weak(live);
            rt.heap.drop_weak(dead);
            rt.heap.drop_global(root);
        });
    }
}

/// A compaction moves values inside the table with no `&Heap` (`props::own_entry`), through
/// the barrier slow path. Here the table is a large object (600 entries, 14 KiB) and old (it
/// survived a GC); a young object goes into p500, 100 holes come before it, and an enumeration
/// moves it 2,400 bytes down, to another 512-byte card. A minor GC then finds it only through
/// the card the move dirtied: the object survives and the verifier passes.
#[test]
fn a_compaction_move_in_an_old_large_table_is_a_barriered_store() {
    on_big_stack(|| {
        let mut c = verified(false);
        c.heap.record_conservative_roots = true;
        // S1 (the default since M5-P2): a minor GC reads the cards of an old large object.
        c.heap.generational = true;
        let iso = isolate(c);
        let rt = &*iso.rt;
        let o = dict_with(rt, 600);
        let root = rt.heap.new_global(val(o));
        assert_eq!(state(o).0, 1024);
        rt.heap.collect();
        for i in 0..100 {
            assert!(del(rt, o, &format!("p{i}")));
        }
        let (young, addr) = hold_new_object(rt, o, "p500");
        // The enumeration: its first `own_entry` compacts.
        assert_eq!(props::own_count(o), 500);
        assert_eq!(props::own_entry(o, 0).0, k(rt, "p100"));
        assert_eq!(state(o), (1024, 500, 500, 2048));
        let t = table(o);
        assert_eq!(object::rd(t, props_index::entry(400)), k(rt, "p500"));
        scrub_stack();
        rt.heap.collect_minor();
        let css = rt.heap.last_conservative_roots();
        assert!(
            !css.iter().any(|c| c.addr() == addr ^ MASK),
            "the conservative scan kept the young object: the check proves nothing"
        );
        let v = rt.heap.weak(young).expect("the moved young value survives");
        assert_eq!(own(rt, o, "p500"), Some(v.to_bits()));
        iso.verify_heap().unwrap();
        rt.heap.drop_weak(young);
        rt.heap.drop_global(root);
    });
}

// ---- many properties --------------------------------------------------------------------------

/// 1,021 properties (one past `MAX_FAST_PROPERTIES`, so the 1,021st add makes the dictionary),
/// then deletes in a random order (fixed seed): after each delete the deleted key is gone and
/// sampled live keys are found; every 64 deletes (with holes in between) the enumeration order
/// is the insertion order of the live keys. At 8 live keys the index goes away; the last
/// delete leaves an empty table; new keys go in again. Run it with `EP9_HASH_SEED=1` for the
/// same buckets on every run (the plan's acceptance form).
#[test]
fn many_properties_deleted_in_random_order() {
    each(|_, rt| {
        let n = MAX_FAST_PROPERTIES + 1;
        let o = object::new_plain_object(rt);
        // The keys live in a JS array (rule 8: no Rust collection holds a Value).
        let all = object::new_array(rt, n);
        for i in 0..n {
            let key = k(rt, &format!("q{i}"));
            object::array_set(rt, all, i as u32, key);
            props::own_define(rt, o, key, int(i as i32), attr::DEFAULT);
            if i + 1 == MAX_FAST_PROPERTIES {
                assert!(!shape::is_dictionary(shape::shape_of(o)));
            }
        }
        assert_eq!(state(o), (1024, n, n, 2048));
        let key = |i: usize| object::array_get(all, i as u32).unwrap();
        let mut live: Vec<usize> = (0..n).collect();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut deletes = 0;
        while !live.is_empty() {
            let at = rng.below(live.len());
            let gone = live.remove(at);
            assert_eq!(props::own_delete(rt, o, key(gone)), Some(attr::DEFAULT));
            deletes += 1;
            assert_eq!(props::own_get(o, key(gone)), None);
            if let Some(&s) = live.get(rng.below(live.len().max(1))) {
                assert_eq!(
                    props::own_get(o, key(s)),
                    Some((int(s as i32), attr::DEFAULT))
                );
            }
            if let (Some(&first), Some(&last)) = (live.first(), live.last()) {
                assert_eq!(
                    props::own_get(o, key(first)).map(|x| x.0),
                    Some(int(first as i32))
                );
                assert_eq!(
                    props::own_get(o, key(last)).map(|x| x.0),
                    Some(int(last as i32))
                );
            }
            let (cap, count, used, b) = state(o);
            assert_eq!(count, live.len());
            assert_eq!(cap, 1024, "a delete never makes a new table");
            assert_eq!(
                b == 0,
                live.len() <= 8,
                "an index exactly above 8 live entries"
            );
            assert!(used >= count && (b > 0 || used == count));
            if deletes % 64 == 0 || live.len() <= 9 {
                let got: Vec<usize> = (0..props::own_count(o))
                    .map(|i| int32_of(props::own_entry(o, i).1) as usize)
                    .collect();
                assert_eq!(got, live, "the order after {deletes} deletes");
                for (i, &j) in live.iter().enumerate() {
                    assert_eq!(props::own_entry(o, i).0, key(j));
                }
            }
        }
        assert_eq!(state(o), (1024, 0, 0, 0));
        assert!(keys(o).is_empty());
        for i in 0..20 {
            put(rt, o, &format!("r{i}"), int(i));
        }
        assert_eq!(state(o), (1024, 20, 20, 2048));
        assert_eq!(keys(o), names("r", 0..20));
        assert_eq!(own(rt, o, "q0"), None);
    });
}
