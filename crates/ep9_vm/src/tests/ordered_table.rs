//! Tests of ordered hash tables (ORDERED_TABLE, kind 77) and the identity hash (H12-Q1)
//! (owner M3-P1 B-COLLECTIONS). Most run under GC stress (a collection before every
//! allocation, then the heap verifier). JS-level tests are in
//! `crates/ep9_builtins/tests/collections.rs`.
//!
//! | Test | Shows |
//! |---|---|
//! | `a_cursor_follows_a_spec_list_model_over_10000_steps` | inserts, deletes, clears and rehashes against a model of the spec List with a live cursor |
//! | `a_rehash_during_iteration_keeps_the_cursor` | a cursor in an obsolete table moves to the same entry of the new one |
//! | `same_value_zero_keys` | -0 = +0, one NaN, an int32 = its double, strings by content |
//! | `a_dead_weak_key_is_cleared_with_its_value` | the ephemeron trace: a dead key and its value are freed by the same GC, both slots read undefined; a later write keeps its live key |
//! | `weak_map_entries_are_ephemerons` | a value that reaches its own key, a cycle of two entries and a key1 -> value1 -> key2 chain (M4-P2 GC2: the fixpoint) |
//! | `a_strong_key_slot_keeps_the_key` | the negative control: the same steps on a Map keep the key, so the weak test can fail |
//! | `a_rehash_drops_dead_weak_entries` | a rehash of a weak table copies only live keys |
//! | `an_identity_hash_is_stable_hidden_and_not_an_address` | H12-Q1: stored once, flag bit 9, not listed, frozen objects too, the same sequence in two isolates |

use std::hint::black_box;

use ep9_heap::{GlobalHandle, WeakHandle};

use super::{config, isolate, on_big_stack};
use crate::abi::ObjectClass;
use crate::frame::{HOLE, UNDEFINED, int32_bits};
use crate::kinds::{self, FLAG_HASH_PRESENT};
use crate::object::{self, cell};
use crate::ordered_table::{self as table, MAP_WIDTH, SET_WIDTH};
use crate::runtime::{Root, Runtime, val};
use crate::{Dispatch, Isolate, conv, property, string};

/// Bytes of zeros between the test's frame and the frames that make the cells (the pattern
/// of tests/layout3.rs; tests/mod.rs is frozen, so each file has its own copy).
const DEEP: usize = 64 << 10;

/// Overwrites 256 KiB of stack below the caller.
#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    black_box(&mut words);
}

/// Runs `made` with [`DEEP`] bytes of zeros between its frames and the caller's.
#[inline(never)]
fn below_a_pad<R>(made: impl FnOnce() -> R) -> R {
    let mut pad = [0u8; DEEP];
    black_box(&mut pad);
    let r = made();
    black_box(&mut pad);
    r
}

/// A new collection object of `class` (a SLOT_OBJECT with an empty table in slot 0), as
/// `Cx::new_collection` makes it. GC point.
fn new_collection(rt: &Runtime, class: ObjectClass) -> u64 {
    let (width, weak) = match class {
        ObjectClass::Map => (MAP_WIDTH, false),
        ObjectClass::Set => (SET_WIDTH, false),
        ObjectClass::WeakMap => (MAP_WIDTH, true),
        _ => (SET_WIDTH, true),
    };
    let o = object::new_slot_object(rt, rt.root(Root::ObjectPrototype), class.id(), 1);
    let t = table::new_empty(rt, width, weak);
    object::slot_set(rt, o, table::TABLE_SLOT, t);
    o
}

/// The value of `k` in the collection `o`, if `k` is a key.
fn get(rt: &Runtime, o: u64, k: u64) -> Option<u64> {
    let t = table::table_of(o);
    table::lookup(rt, t, k).map(|i| table::value_at(t, i))
}

fn num(d: f64) -> u64 {
    conv::num(d)
}

/// A small deterministic PRNG (xorshift64*), for the model test.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// The spec's view of a Map: a List of entries in insertion order, where delete and clear
/// set an entry to empty (`None`) and never shrink the List; a walk keeps an index into it
/// (§24.1.3.5 forEach, §24.1.5.1 CreateMapIterator).
#[derive(Default)]
struct Model {
    list: Vec<Option<(i32, i32)>>,
}

impl Model {
    fn set(&mut self, k: i32, v: i32) {
        match self.list.iter_mut().flatten().find(|e| e.0 == k) {
            Some(e) => e.1 = v,
            None => self.list.push(Some((k, v))),
        }
    }

    fn delete(&mut self, k: i32) -> bool {
        match self.list.iter().position(|e| e.is_some_and(|e| e.0 == k)) {
            Some(i) => {
                self.list[i] = None;
                true
            }
            None => false,
        }
    }

    fn clear(&mut self) {
        self.list.fill(None);
    }

    fn next(&self, index: &mut usize) -> Option<(i32, i32)> {
        while *index < self.list.len() {
            let e = self.list[*index];
            *index += 1;
            if e.is_some() {
                return e;
            }
        }
        None
    }

    fn live(&self) -> usize {
        self.list.iter().flatten().count()
    }
}

/// 10,000 random steps (set, delete, clear, cursor step) on a Map of small int keys, with
/// two live cursors, under GC stress and the heap verifier: the table and its cursors give
/// what the spec List gives, also across the rehashes (grow, compaction, shrink) and clears.
#[test]
fn a_cursor_follows_a_spec_list_model_over_10000_steps() {
    on_big_stack(|| {
        let iso = isolate(config(Dispatch::DEFAULT, true));
        let rt: &Runtime = &iso.rt;
        let map = new_collection(rt, ObjectClass::Map);
        let h = rt.heap.new_global(val(map));
        let mut model = Model::default();
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        // Two cursors: (table, index) here, the model's index.
        let mut cursors = [(table::table_of(map), 0usize, 0usize); 2];
        let mut rehashes = 0;
        for step in 0..10_000 {
            let before = table::table_of(map);
            match rng.below(100) {
                0..=54 => {
                    let k = rng.below(400) as i32;
                    table::set(rt, map, num(f64::from(k)), num(f64::from(step))).unwrap();
                    model.set(k, step);
                }
                55..=84 => {
                    let k = rng.below(400) as i32;
                    assert_eq!(
                        table::delete(rt, map, num(f64::from(k))),
                        model.delete(k),
                        "delete {k} at step {step}"
                    );
                }
                85 => {
                    table::clear(rt, map);
                    model.clear();
                }
                _ => {
                    let c = &mut cursors[rng.below(2) as usize];
                    let got = table::cursor_next(&mut c.0, &mut c.1).map(|i| {
                        let k = val(table::key_at(c.0, i)).as_int32().unwrap();
                        let v = val(table::value_at(c.0, i)).as_int32().unwrap();
                        (k, v)
                    });
                    assert_eq!(got, model.next(&mut c.2), "cursor step at step {step}");
                    if got.is_none() && rng.below(4) == 0 {
                        // A new walk from the start.
                        *c = (table::table_of(map), 0, 0);
                    }
                }
            }
            if table::table_of(map) != before {
                rehashes += 1;
            }
            assert_eq!(table::live_count(table::table_of(map)), model.live());
        }
        assert!(rehashes > 20, "only {rehashes} rehashes and clears");
        // Every key the model has, with its value.
        for e in model.list.iter().flatten() {
            assert_eq!(
                get(rt, map, num(f64::from(e.0))),
                Some(int32_bits(e.1)),
                "key {}",
                e.0
            );
        }
        iso.verify_heap().unwrap();
        rt.heap.drop_global(h);
    });
}

/// A cursor parked at entry 1 of a table with a deleted entry 0 keeps its place across two
/// rehashes (the old tables forward), and sees the entries added after it.
#[test]
fn a_rehash_during_iteration_keeps_the_cursor() {
    for stress in [false, true] {
        on_big_stack(move || {
            let iso = isolate(config(Dispatch::DEFAULT, stress));
            let rt: &Runtime = &iso.rt;
            let set = new_collection(rt, ObjectClass::Set);
            let h = rt.heap.new_global(val(set));
            for k in 0..3 {
                table::set(rt, set, num(f64::from(k)), UNDEFINED).unwrap();
            }
            let (mut t, mut i) = (table::table_of(set), 0);
            let first = table::cursor_next(&mut t, &mut i).unwrap();
            assert_eq!(table::key_at(t, first), num(0.0));
            assert!(table::delete(rt, set, num(0.0)));
            let old = table::table_of(set);
            for k in 3..40 {
                table::set(rt, set, num(f64::from(k)), UNDEFINED).unwrap();
            }
            assert!(table::delete(rt, set, num(5.0)));
            assert_ne!(table::table_of(set), old, "the inserts rehashed the table");
            let mut rest = Vec::new();
            while let Some(e) = table::cursor_next(&mut t, &mut i) {
                rest.push(val(table::key_at(t, e)).as_int32().unwrap());
            }
            let want: Vec<i32> = (1..40).filter(|&k| k != 5).collect();
            assert_eq!(rest, want);
            assert_eq!(
                t,
                table::table_of(set),
                "the cursor reached the current table"
            );
            // At the end it stays there: an entry added later is still seen.
            table::set(rt, set, num(99.0), UNDEFINED).unwrap();
            let e = table::cursor_next(&mut t, &mut i).unwrap();
            assert_eq!(table::key_at(t, e), num(99.0));
            iso.verify_heap().unwrap();
            rt.heap.drop_global(h);
        });
    }
}

/// SameValueZero keys: -0 is stored as +0, every NaN is one key, a number has one stored
/// form, and two strings with the same code units (not atoms) are one key.
#[test]
fn same_value_zero_keys() {
    on_big_stack(|| {
        let iso = isolate(config(Dispatch::DEFAULT, true));
        let rt: &Runtime = &iso.rt;
        let map = new_collection(rt, ObjectClass::Map);
        let h = rt.heap.new_global(val(map));
        table::set(rt, map, num(-0.0), num(1.0)).unwrap();
        assert_eq!(table::key_at(table::table_of(map), 0), int32_bits(0));
        assert_eq!(get(rt, map, num(0.0)), Some(num(1.0)));
        let nan = ep9_value::Value::double(f64::NAN).to_bits();
        table::set(rt, map, nan, num(2.0)).unwrap();
        assert_eq!(get(rt, map, num(f64::NAN)), Some(num(2.0)));
        // 7 as a double Value finds the int32 key 7.
        table::set(rt, map, num(7.0), num(3.0)).unwrap();
        let seven = ep9_value::Value::double(7.0).to_bits();
        assert_ne!(seven, num(7.0));
        assert_eq!(get(rt, map, seven), Some(num(3.0)));
        let a = object::new_string(rt, "a key text");
        table::set(rt, map, a, num(4.0)).unwrap();
        let b = object::new_string(rt, "a key text");
        assert_ne!(a, b);
        assert!(!string::is_atom(a) && !string::is_atom(b));
        assert_eq!(get(rt, map, b), Some(num(4.0)));
        assert_eq!(get(rt, map, object::new_string(rt, "a key tex")), None);
        assert_eq!(table::live_count(table::table_of(map)), 4);
        iso.verify_heap().unwrap();
        rt.heap.drop_global(h);
    });
}

/// The handles of [`weak_entry`].
struct WeakEntry {
    map: GlobalHandle,
    key: WeakHandle,
    value: WeakHandle,
}

/// A collection of `class` with one entry (a fresh key object, a fresh value object), made
/// below a pad so that no stack word keeps the key or the value: the map is rooted by a
/// global handle, the key and the value only by the table (and weak handles).
#[inline(never)]
fn weak_entry(rt: &Runtime, class: ObjectClass) -> WeakEntry {
    below_a_pad(|| {
        let map = new_collection(rt, class);
        let key = object::new_plain_object(rt);
        let value = object::new_plain_object(rt);
        table::set(rt, map, key, value).unwrap();
        WeakEntry {
            map: rt.heap.new_global(val(map)),
            key: rt.heap.new_weak(val(key)),
            value: rt.heap.new_weak(val(value)),
        }
    })
}

/// A write into the map of `e`: a new entry whose key a global handle keeps (no rehash: the
/// table has room for it).
#[inline(never)]
fn write_one(rt: &Runtime, e: &WeakEntry) -> GlobalHandle {
    below_a_pad(|| {
        let map = rt.heap.global(e.map).to_bits();
        let k = object::new_plain_object(rt);
        let h = rt.heap.new_global(val(k));
        let t = table::table_of(map);
        table::set(rt, map, k, UNDEFINED).unwrap();
        assert_eq!(table::table_of(map), t, "no rehash");
        h
    })
}

/// True if the cell of the weak handle `h` is alive. Read below a pad, so the cell's bits do
/// not stay in the caller's frame for the next stack scan.
#[inline(never)]
fn alive(rt: &Runtime, h: WeakHandle) -> bool {
    below_a_pad(|| black_box(rt.heap.weak(h)).is_some())
}

/// Collects after a stack scrub, so that only real references keep cells.
#[inline(never)]
fn collect(iso: &Isolate) {
    scrub_stack();
    iso.rt.heap.collect();
}

/// The weak trace (the B-COLLECTIONS acceptance, with the M4-P2 ephemeron fixpoint): a
/// WeakMap key that only the table holds is cleared to undefined by `collect_garbage`, and
/// its value, which only that entry holds, dies in the same GC (the value is traced only
/// while its key is live) and its slot is cleared. With GC stress and the heap verifier on,
/// a dangling slot fails the verifier.
#[test]
fn a_dead_weak_key_is_cleared_with_its_value() {
    for stress in [false, true] {
        on_big_stack(move || {
            let iso = isolate(config(Dispatch::DEFAULT, stress));
            let e = weak_entry(&iso.rt, ObjectClass::WeakMap);
            collect(&iso);
            let rt: &Runtime = &iso.rt;
            assert!(!alive(rt, e.key), "the key died (stress={stress})");
            assert!(!alive(rt, e.value), "its value died in the same GC");
            let t = table::table_of(rt.heap.global(e.map).to_bits());
            assert_eq!(table::key_at(t, 0), UNDEFINED, "the key slot is cleared");
            assert_eq!(
                table::value_at(t, 0),
                UNDEFINED,
                "the value slot is cleared"
            );
            // `WeakMap.prototype.get(undefined)` does not find the cleared entry.
            assert_eq!(table::lookup(rt, t, UNDEFINED), None);
            let k2 = write_one(rt, &e);
            collect(&iso);
            let rt: &Runtime = &iso.rt;
            let t = table::table_of(rt.heap.global(e.map).to_bits());
            assert_ne!(table::key_at(t, 1), UNDEFINED, "the live key stays");
            iso.verify_heap().unwrap();
            rt.heap.drop_global(k2);
            rt.heap.drop_global(e.map);
        });
    }
}

/// The handles of [`ephemeron_graph`]: the WeakMap and weak handles of its keys and values.
struct EphemeronGraph {
    map: GlobalHandle,
    cells: Vec<WeakHandle>,
}

/// A WeakMap with one entry per (key, value) pair of `links`: entry i maps a fresh key object
/// k_i to a Map v_i, and v_i holds k_j (as a Map key) for each `j` in `links[i]`. Made below a
/// pad; `root` keeps k_0 by a global handle.
#[inline(never)]
fn ephemeron_graph(
    rt: &Runtime,
    links: &[&[usize]],
    root: bool,
) -> (EphemeronGraph, Option<GlobalHandle>) {
    below_a_pad(|| {
        let map = new_collection(rt, ObjectClass::WeakMap);
        let hm = rt.heap.new_global(val(map));
        let keys: Vec<GlobalHandle> = links
            .iter()
            .map(|_| rt.heap.new_global(val(object::new_plain_object(rt))))
            .collect();
        let mut cells = Vec::new();
        // Insert in reverse order: the trace meets the end of a chain first.
        for (i, to) in links.iter().enumerate().rev() {
            let v = new_collection(rt, ObjectClass::Map);
            for &j in *to {
                table::set(rt, v, rt.heap.global(keys[j]).to_bits(), UNDEFINED).unwrap();
            }
            let k = rt.heap.global(keys[i]).to_bits();
            table::set(rt, rt.heap.global(hm).to_bits(), k, v).unwrap();
            cells.push(rt.heap.new_weak(val(k)));
            cells.push(rt.heap.new_weak(val(v)));
        }
        let r = root.then(|| rt.heap.new_global(rt.heap.global(keys[0])));
        for k in keys {
            rt.heap.drop_global(k);
        }
        (EphemeronGraph { map: hm, cells }, r)
    })
}

/// WeakMap entries are ephemerons (M4-P2 GC2): a value is traced only once its key is live
/// by another path. A value that reaches its own key, and a cycle of two entries, die with
/// their keys; a chain key1 -> value1 -> key2 -> value2 -> key3 lives while key1 is rooted
/// and dies when it is not. With and without GC stress (heap verifier on).
#[test]
fn weak_map_entries_are_ephemerons() {
    let own: &[&[usize]] = &[&[0]];
    let cycle: &[&[usize]] = &[&[1], &[0]];
    let chain: &[&[usize]] = &[&[1], &[2], &[]];
    for stress in [false, true] {
        for (links, root) in [
            (own, false),
            (cycle, false),
            (chain, false),
            (chain, true),
            (cycle, true),
        ] {
            on_big_stack(move || {
                let iso = isolate(config(Dispatch::DEFAULT, stress));
                let (g, r) = ephemeron_graph(&iso.rt, links, root);
                collect(&iso);
                let rt: &Runtime = &iso.rt;
                for (n, &h) in g.cells.iter().enumerate() {
                    assert_eq!(
                        alive(rt, h),
                        root,
                        "cell {n} of {links:?} (root {root}, stress {stress})"
                    );
                }
                iso.verify_heap().unwrap();
                if let Some(r) = r {
                    rt.heap.drop_global(r);
                    collect(&iso);
                    let rt: &Runtime = &iso.rt;
                    assert!(
                        g.cells.iter().all(|&h| !alive(rt, h)),
                        "all die without the root"
                    );
                    iso.verify_heap().unwrap();
                }
                iso.rt.heap.drop_global(g.map);
            });
        }
    }
}

/// The negative control of the weak test: the same steps with a Map (strong key slots) keep
/// the key and the value, so `a_dead_weak_key_is_cleared_with_its_value` fails if the
/// trace visits a weak table's keys strongly.
#[test]
fn a_strong_key_slot_keeps_the_key() {
    on_big_stack(|| {
        let iso = isolate(config(Dispatch::DEFAULT, false));
        let e = weak_entry(&iso.rt, ObjectClass::Map);
        collect(&iso);
        collect(&iso);
        let rt: &Runtime = &iso.rt;
        assert!(alive(rt, e.key), "a Map keeps its key");
        assert!(alive(rt, e.value), "and its value");
        let t = table::table_of(rt.heap.global(e.map).to_bits());
        assert_ne!(table::key_at(t, 0), UNDEFINED);
        rt.heap.drop_global(e.map);
    });
}

/// A rehash of a weak table drops the entries whose keys died (and the deleted ones); the
/// live keys keep their values.
#[test]
fn a_rehash_drops_dead_weak_entries() {
    on_big_stack(|| {
        let iso = isolate(config(Dispatch::DEFAULT, false));
        let e = weak_entry(&iso.rt, ObjectClass::WeakSet);
        collect(&iso);
        let rt: &Runtime = &iso.rt;
        assert!(!alive(rt, e.key));
        let set = rt.heap.global(e.map).to_bits();
        let mut keep = Vec::new();
        for _ in 0..20 {
            let k = object::new_plain_object(rt);
            keep.push(rt.heap.new_global(val(k)));
            table::set(rt, set, k, UNDEFINED).unwrap();
        }
        let t = table::table_of(set);
        assert_eq!(table::live_count(t), 20, "the dead entry is gone");
        assert_ne!(table::key_at(t, 0), UNDEFINED);
        for h in &keep {
            let k = rt.heap.global(*h).to_bits();
            assert!(table::lookup(rt, t, k).is_some());
        }
        iso.verify_heap().unwrap();
        for h in keep {
            rt.heap.drop_global(h);
        }
        rt.heap.drop_global(e.map);
    });
}

/// The identity hashes of `n` new objects in a new isolate (with extra allocations between
/// them when `shift`, so the addresses differ from a run without).
fn hashes(n: usize, shift: bool) -> Vec<u32> {
    on_big_stack(move || {
        let iso = isolate(config(Dispatch::DEFAULT, false));
        let rt: &Runtime = &iso.rt;
        (0..n)
            .map(|_| {
                if shift {
                    object::new_plain_object(rt);
                }
                let o = object::new_plain_object(rt);
                table::identity_hash(rt, o, true).unwrap()
            })
            .collect()
    })
}

/// H12-Q1 (the H12 review row 4 output): an object gets its identity hash on first use: 30
/// bits from the isolate's PRNG, stored once (the same hash after GCs), marked by word0 flag
/// bit 9, under a private name that `[[OwnPropertyKeys]]` does not list, also on a frozen
/// object; a lookup does not make one. Two isolates give the same hashes in the same order
/// although their objects sit at other addresses (no address goes into a hash, H12-Q2).
#[test]
fn an_identity_hash_is_stable_hidden_and_not_an_address() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let rt: &Runtime = &iso.rt;
        let o = object::new_plain_object(rt);
        let ho = rt.heap.new_global(val(o));
        // SAFETY: `o` is a live object cell (a global handle or a Rust local holds it).
        let flags = |o: u64| unsafe { cell(o).header() }.flags();
        assert_eq!(flags(o) & FLAG_HASH_PRESENT, 0);
        assert_eq!(
            table::identity_hash(rt, o, false),
            None,
            "a lookup makes none"
        );
        assert_eq!(flags(o) & FLAG_HASH_PRESENT, 0);
        let h = table::identity_hash(rt, o, true).unwrap();
        assert!(h < 1 << 30);
        assert_ne!(flags(o) & FLAG_HASH_PRESENT, 0);
        iso.collect_garbage();
        let rt: &Runtime = &iso.rt;
        let o = rt.heap.global(ho).to_bits();
        assert_eq!(table::identity_hash(rt, o, true), Some(h), "stored once");
        assert_eq!(table::identity_hash(rt, o, false), Some(h));
        // Not listed: the object still has no own keys.
        let keys = iso.with_cx(|cx| {
            let o = crate::abi::Local::<crate::abi::JsObject>::from_bits(o);
            let keys = cx.own_property_keys(o)?;
            let len = cx.get(keys, "length")?;
            cx.to_number(len)
        });
        assert_eq!(keys.unwrap(), 0.0);
        // A non-extensible object takes a hash too.
        let rt: &Runtime = &iso.rt;
        let f = object::new_plain_object(rt);
        property::prevent_extensions(rt, f);
        assert!(!property::is_extensible(f));
        assert!(table::identity_hash(rt, f, true).is_some());
        assert!(!property::is_extensible(f));
        assert!(kinds::is_object(f));
        iso.verify_heap().unwrap();
        iso.rt.heap.drop_global(ho);
    });
    let a = hashes(8, false);
    assert_eq!(a, hashes(8, true), "the same hashes at other addresses");
    assert_eq!(a, hashes(8, false));
    let mut unique = a.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), a.len());
}

/// Keys of a deleted entry hold HOLE (never a JS value), so a lookup of undefined in a Map
/// does not find a deleted entry.
#[test]
fn a_deleted_entry_is_a_hole_not_undefined() {
    on_big_stack(|| {
        let iso = isolate(config(Dispatch::DEFAULT, false));
        let rt: &Runtime = &iso.rt;
        let map = new_collection(rt, ObjectClass::Map);
        let h = rt.heap.new_global(val(map));
        table::set(rt, map, num(1.0), num(1.0)).unwrap();
        assert!(table::delete(rt, map, num(1.0)));
        assert_eq!(table::key_at(table::table_of(map), 0), HOLE);
        assert_eq!(get(rt, map, UNDEFINED), None);
        table::set(rt, map, UNDEFINED, num(2.0)).unwrap();
        assert_eq!(get(rt, map, UNDEFINED), Some(num(2.0)));
        rt.heap.drop_global(h);
    });
}
