//! Ephemerons (plan v9 §4.2 task 1, GFF-EPH): [`Tracer::visit_ephemeron`] and the fixpoint.
//!
//! ```text
//! EPH test cell: word0, meta n, then n entries (key slot, value slot), traced with
//!   visit_ephemeron(key, value): the key is weak; the value is traced once the key is marked
//! cases: a value that reaches its own key; a cycle of entries; a chain key1 -> value1 ->
//!   key2 -> ... met in the wrong order (rounds, then the linear form); keys that are not
//!   cells; a value live by another path; a random graph against a model fixpoint
//! ```
//!
//! Each case runs twice: a plain mark, and a checked mark (`verify_after_gc`, verify.rs).

use std::collections::BTreeSet;

use ep9_value::Value;

use super::*;
use crate::GlobalHandle;
use crate::trace::EPHEMERON_ROUNDS;

/// The ephemeron table kind of these tests.
const EPH: KindId = KindId::new(15).unwrap();
const EPH_HEADER: usize = 16;

fn eph_entries(c: CellPtr) -> usize {
    // SAFETY: the heap calls size and trace functions only with live EPH cells, whose meta
    // word is written right after allocation.
    unsafe { c.field::<u64>(8).read() as usize }
}

fn eph_size(c: CellPtr) -> usize {
    EPH_HEADER + 16 * eph_entries(c)
}

fn eph_trace(c: CellPtr, t: &mut Tracer<'_>) {
    for i in 0..eph_entries(c) {
        // SAFETY: a live EPH cell has a (key, value) pair of Value slots per entry.
        unsafe { t.visit_ephemeron(c.slot(key_at(i)), c.slot(key_at(i) + 8)) };
    }
}

const fn key_at(i: usize) -> usize {
    EPH_HEADER + 16 * i
}

/// A test heap with the EPH kind, `checked`: a checked mark (verify after every GC).
fn eph_heap(checked: bool) -> Heap {
    let heap = test_heap(|c| c.verify_after_gc = checked);
    heap.register_kind(
        EPH,
        CellKind {
            name: "test-ephemeron-table",
            size: CellSize::Dynamic(eph_size),
            trace: eph_trace,
        },
    )
    .unwrap();
    heap
}

/// A new EPH table of `n` entries, every key and value `undefined`.
fn new_table(heap: &Heap, n: usize) -> CellPtr {
    let c = heap.alloc(EPH, EPH_HEADER + 16 * n).expect("alloc table");
    // SAFETY: a new EPH cell; +8 is its meta word, written before any GC point.
    unsafe { c.field::<u64>(8).write(n as u64) };
    c
}

fn entry_slots(t: CellPtr, i: usize) -> (&'static Slot, &'static Slot) {
    assert!(i < eph_entries(t));
    // SAFETY: entry i of a live EPH cell is two Value slots; the test keeps `t` alive.
    unsafe { (t.slot(key_at(i)), t.slot(key_at(i) + 8)) }
}

fn set_entry(heap: &Heap, t: CellPtr, i: usize, k: Value, v: Value) {
    let (ks, vs) = entry_slots(t, i);
    // SAFETY: `t` is a live table (caller).
    let h = unsafe { t.header() };
    ks.set(h, k, heap.barrier());
    vs.set(h, v, heap.barrier());
}

fn entry(t: CellPtr, i: usize) -> (u64, u64) {
    let (k, v) = entry_slots(t, i);
    (k.bits(), v.bits())
}

/// A full collection with precise roots only (the conservative scan reads no words).
fn precise_gc(heap: &Heap) {
    heap.collect_with_words(&[]);
    assert!(heap.last_conservative_roots().is_empty());
}

fn last_fixpoint(heap: &Heap) -> (u32, bool) {
    heap.inner.borrow().last_ephemeron
}

const UNDEF: u64 = Value::UNDEFINED.to_bits();

/// A value that refers to its own key keeps neither alive (the M3-P1-WEAK-GAP case).
#[test]
fn a_value_that_reaches_its_own_key_does_not_keep_it() {
    for checked in [false, true] {
        let heap = eph_heap(checked);
        let t = new_table(&heap, 1);
        let g = heap.new_global(t.to_value());
        let (k, v) = (new_node(&heap, 1, 0), new_node(&heap, 2, 1));
        store(&heap, v, 0, k.to_value());
        set_entry(&heap, t, 0, k.to_value(), v.to_value());
        let (ka, va) = (k.addr(), v.addr());
        precise_gc(&heap);
        assert_eq!(entry(t, 0), (UNDEF, UNDEF), "key and value cleared");
        assert!(!heap.is_cell(ka) && !heap.is_cell(va), "both freed");
        heap.verify().expect("verifier");
        heap.drop_global(g);
    }
}

/// The negative control: the same graph in a strong NODE keeps both, so the test above can
/// fail.
#[test]
fn a_strong_pair_keeps_its_own_key() {
    let heap = eph_heap(false);
    let t = new_node(&heap, 9, 2);
    let g = heap.new_global(t.to_value());
    let (k, v) = (new_node(&heap, 1, 0), new_node(&heap, 2, 1));
    store(&heap, v, 0, k.to_value());
    store(&heap, t, 0, k.to_value());
    store(&heap, t, 1, v.to_value());
    precise_gc(&heap);
    assert!(heap.is_cell(k.addr()) && heap.is_cell(v.addr()));
    heap.drop_global(g);
}

/// A cycle of entries: value 1 reaches key 2 and value 2 reaches key 1. With no key live
/// from outside, all of it dies; with key 1 rooted, all of it lives.
#[test]
fn an_ephemeron_cycle() {
    for checked in [false, true] {
        for rooted in [false, true] {
            let heap = eph_heap(checked);
            let t = new_table(&heap, 2);
            let g = heap.new_global(t.to_value());
            let ks = [new_node(&heap, 1, 0), new_node(&heap, 2, 0)];
            let vs = [new_node(&heap, 3, 1), new_node(&heap, 4, 1)];
            store(&heap, vs[0], 0, ks[1].to_value());
            store(&heap, vs[1], 0, ks[0].to_value());
            set_entry(&heap, t, 0, ks[0].to_value(), vs[0].to_value());
            set_entry(&heap, t, 1, ks[1].to_value(), vs[1].to_value());
            let root = rooted.then(|| heap.new_global(ks[0].to_value()));
            let addrs: Vec<usize> = ks.iter().chain(&vs).map(|c| c.addr()).collect();
            precise_gc(&heap);
            for &a in &addrs {
                assert_eq!(heap.is_cell(a), rooted, "rooted {rooted} checked {checked}");
            }
            if rooted {
                assert_eq!(
                    entry(t, 0),
                    (ks[0].to_value().to_bits(), vs[0].to_value().to_bits())
                );
                assert_eq!(
                    entry(t, 1),
                    (ks[1].to_value().to_bits(), vs[1].to_value().to_bits())
                );
            } else {
                assert_eq!((entry(t, 0), entry(t, 1)), ((UNDEF, UNDEF), (UNDEF, UNDEF)));
            }
            heap.verify().expect("verifier");
            if let Some(r) = root {
                heap.drop_global(r);
            }
            heap.drop_global(g);
        }
    }
}

/// A chain key1 -> value1 -> key2 -> value2 -> ... of `depth` entries, stored in reverse
/// order so that each round of the fixpoint resolves one entry. Returns the heap, the
/// table, the keys and the values.
fn chain(checked: bool, depth: usize) -> (Heap, CellPtr, Vec<CellPtr>, Vec<CellPtr>) {
    let heap = eph_heap(checked);
    let t = new_table(&heap, depth);
    let keys: Vec<CellPtr> = (0..depth).map(|i| new_node(&heap, i as u32, 0)).collect();
    let values: Vec<CellPtr> = (0..depth)
        .map(|i| new_node(&heap, 1000 + i as u32, 1))
        .collect();
    for i in 0..depth {
        if i + 1 < depth {
            store(&heap, values[i], 0, keys[i + 1].to_value());
        }
        // Entry 0 holds the last key: the drain meets the chain from its end.
        let e = depth - 1 - i;
        set_entry(&heap, t, e, keys[i].to_value(), values[i].to_value());
    }
    (heap, t, keys, values)
}

/// The acceptance chain key1 -> value1 -> key2: with key 1 rooted every link lives, with it
/// unrooted every link dies. A chain of up to `EPHEMERON_ROUNDS + 1` entries ends in rounds;
/// a longer one ends in the linear form.
#[test]
fn a_key1_value1_key2_chain() {
    for checked in [false, true] {
        let rounds_max = EPHEMERON_ROUNDS as usize;
        for depth in [2, 3, rounds_max + 1, rounds_max + 2, 64] {
            let (heap, t, keys, values) = chain(checked, depth);
            let g = heap.new_global(t.to_value());
            let root = heap.new_global(keys[0].to_value());
            precise_gc(&heap);
            for i in 0..depth {
                assert!(heap.is_cell(keys[i].addr()) && heap.is_cell(values[i].addr()));
                let e = depth - 1 - i;
                assert_eq!(
                    entry(t, e),
                    (keys[i].to_value().to_bits(), values[i].to_value().to_bits())
                );
            }
            // One entry per round: the first drain marks key 1, and each round marks the
            // next key only through the drain after it.
            let (rounds, linear) = last_fixpoint(&heap);
            assert_eq!(linear, depth > rounds_max + 1, "depth {depth}");
            assert_eq!(
                rounds as usize,
                (depth - 1).min(rounds_max),
                "depth {depth}"
            );
            heap.verify().expect("verifier");
            let addrs: Vec<usize> = keys.iter().chain(&values).map(|c| c.addr()).collect();
            heap.drop_global(root);
            precise_gc(&heap);
            assert!(
                addrs.iter().all(|&a| !heap.is_cell(a)),
                "depth {depth}: all die"
            );
            assert!((0..depth).all(|e| entry(t, e) == (UNDEF, UNDEF)));
            heap.verify().expect("verifier");
            heap.drop_global(g);
        }
    }
}

/// A chain over two tables: an entry of table A whose value is a key of table B, and the
/// other way round.
#[test]
fn a_chain_across_two_tables() {
    for checked in [false, true] {
        let heap = eph_heap(checked);
        let (a, b) = (new_table(&heap, 1), new_table(&heap, 1));
        let ga = heap.new_global(a.to_value());
        let gb = heap.new_global(b.to_value());
        let (k1, v1, v2) = (
            new_node(&heap, 1, 0),
            new_node(&heap, 2, 0),
            new_node(&heap, 3, 0),
        );
        // B: (v1 -> v2) is met first; A: (k1 -> v1).
        set_entry(&heap, b, 0, v1.to_value(), v2.to_value());
        set_entry(&heap, a, 0, k1.to_value(), v1.to_value());
        let root = heap.new_global(k1.to_value());
        precise_gc(&heap);
        assert!(heap.is_cell(v1.addr()) && heap.is_cell(v2.addr()));
        heap.drop_global(root);
        let all = [k1.addr(), v1.addr(), v2.addr()];
        precise_gc(&heap);
        assert!(all.iter().all(|&x| !heap.is_cell(x)));
        assert_eq!((entry(a, 0), entry(b, 0)), ((UNDEF, UNDEF), (UNDEF, UNDEF)));
        heap.drop_global(ga);
        heap.drop_global(gb);
    }
}

/// A key that is not a cell keeps its value (as a strong slot does); a dead key whose value
/// is live by another path clears only the key.
#[test]
fn non_cell_keys_and_values_live_elsewhere() {
    for checked in [false, true] {
        let heap = eph_heap(checked);
        let t = new_table(&heap, 3);
        let g = heap.new_global(t.to_value());
        let (v0, k1, v1) = (
            new_node(&heap, 1, 0),
            new_node(&heap, 2, 0),
            new_node(&heap, 3, 0),
        );
        set_entry(&heap, t, 0, Value::int32(7), v0.to_value());
        set_entry(&heap, t, 1, k1.to_value(), v1.to_value());
        set_entry(&heap, t, 2, k1.to_value(), Value::int32(5));
        let keep_v1 = heap.new_global(v1.to_value());
        let k1a = k1.addr();
        precise_gc(&heap);
        assert!(heap.is_cell(v0.addr()), "a non-cell key keeps its value");
        assert_eq!(
            entry(t, 0),
            (Value::int32(7).to_bits(), v0.to_value().to_bits())
        );
        assert!(!heap.is_cell(k1a), "the key died");
        assert_eq!(
            entry(t, 1),
            (UNDEF, v1.to_value().to_bits()),
            "the live value stays"
        );
        assert_eq!(entry(t, 2), (UNDEF, Value::int32(5).to_bits()));
        heap.verify().expect("verifier");
        heap.drop_global(keep_v1);
        heap.drop_global(g);
    }
}

/// A random graph of nodes and ephemeron tables against a model fixpoint: the cells the GC
/// keeps are exactly the ones the model reaches, every entry with a dead key reads
/// (undefined, value or undefined), and every other slot keeps its bits.
#[test]
fn random_graphs_match_a_model_fixpoint() {
    let cases = if cfg!(miri) { 6 } else { 300 };
    for seed in 0..cases {
        random_graph(0xE9E9_0000 + seed, seed % 2 == 1);
    }
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) % n as u64) as usize
    }
}

fn random_graph(seed: u64, checked: bool) {
    let mut rng = Rng(seed);
    let heap = eph_heap(checked);
    let n_nodes = 4 + rng.below(40);
    let n_tables = 1 + rng.below(4);
    // Model: node i has 2 slots (Option<target>); table j has entries (key, value) that are
    // Option<node>.
    let nodes: Vec<CellPtr> = (0..n_nodes).map(|i| new_node(&heap, i as u32, 2)).collect();
    let tables: Vec<CellPtr> = (0..n_tables)
        .map(|_| new_table(&heap, 1 + rng.below(12)))
        .collect();
    let mut node_edges = vec![[None::<usize>; 2]; n_nodes];
    let mut entries: Vec<Vec<(Option<usize>, Option<usize>)>> = Vec::new();
    for (i, edges) in node_edges.iter_mut().enumerate() {
        for (s, e) in edges.iter_mut().enumerate() {
            if rng.below(3) == 0 {
                let to = rng.below(n_nodes);
                *e = Some(to);
                store(&heap, nodes[i], s, nodes[to].to_value());
            }
        }
    }
    for &t in &tables {
        let mut es = Vec::new();
        for e in 0..eph_entries(t) {
            let k = (rng.below(5) > 0).then(|| rng.below(n_nodes));
            let v = (rng.below(5) > 0).then(|| rng.below(n_nodes));
            let kv = k.map_or(Value::int32(e as i32), |k| nodes[k].to_value());
            let vv = v.map_or(Value::NULL, |v| nodes[v].to_value());
            set_entry(&heap, t, e, kv, vv);
            es.push((k, v));
        }
        entries.push(es);
    }
    // Roots: some nodes and some tables.
    let node_roots: Vec<usize> = (0..n_nodes).filter(|_| rng.below(8) == 0).collect();
    let table_roots: Vec<usize> = (0..n_tables).filter(|_| rng.below(3) > 0).collect();
    let handles: Vec<GlobalHandle> = node_roots
        .iter()
        .map(|&i| heap.new_global(nodes[i].to_value()))
        .chain(
            table_roots
                .iter()
                .map(|&j| heap.new_global(tables[j].to_value())),
        )
        .collect();

    // The model fixpoint.
    let mut live: BTreeSet<usize> = BTreeSet::new();
    let mut work: Vec<usize> = node_roots.clone();
    loop {
        while let Some(i) = work.pop() {
            if live.insert(i) {
                work.extend(node_edges[i].iter().flatten());
            }
        }
        for &j in &table_roots {
            for &(k, v) in &entries[j] {
                let key_live = k.is_none_or(|k| live.contains(&k));
                if key_live
                    && let Some(v) = v
                    && !live.contains(&v)
                {
                    work.push(v);
                }
            }
        }
        if work.is_empty() {
            break;
        }
    }

    let addrs: Vec<usize> = nodes.iter().map(|c| c.addr()).collect();
    precise_gc(&heap);
    for (i, &a) in addrs.iter().enumerate() {
        assert_eq!(
            heap.is_cell(a),
            live.contains(&i),
            "seed {seed:#x} node {i}"
        );
    }
    for &j in &table_roots {
        for (e, &(k, v)) in entries[j].iter().enumerate() {
            let (kb, vb) = entry(tables[j], e);
            let key_live = k.is_none_or(|k| live.contains(&k));
            let want_k = match k {
                Some(k) if key_live => nodes[k].to_value().to_bits(),
                Some(_) => UNDEF,
                None => Value::int32(e as i32).to_bits(),
            };
            let want_v = match v {
                Some(v) if live.contains(&v) => nodes[v].to_value().to_bits(),
                Some(_) => UNDEF,
                None => Value::NULL.to_bits(),
            };
            assert_eq!(
                (kb, vb),
                (want_k, want_v),
                "seed {seed:#x} table {j} entry {e}"
            );
        }
    }
    heap.verify().expect("verifier");
    for h in handles {
        heap.drop_global(h);
    }
}
