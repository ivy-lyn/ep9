//! Tests of the whole heap. Per-module unit tests live next to their module.
//!
//! | File | Covers |
//! |---|---|
//! | `barrier.rs` | barrier v2: filter, threshold compare, slow path stores, shape install, init |
//! | `model.rs` | the H11 decision model: exhaustive interleavings, 6 scenarios, negative controls |
//! | `alloc.rs` | TLAB, holes, overflow, LOS, pre-fill, kinds, limits, offsets |
//! | `ephemeron.rs` | `Tracer::visit_ephemeron`: own-key values, cycles, key1 -> value1 -> key2 chains (rounds and the linear form), non-cell keys, random graphs against a model fixpoint |
//! | `gc.rs` | mark-sweep with precise roots, weak slots, lazy sweep, zapping, GC-stress, verifier |
//! | `css.rs` | conservative stack scan: locals, interior and one-past-end pointers, S7 (not Miri) |
//! | `stress.rs` | randomized mutator against a model of the object graph |
//! | `s1.rs` | S1 step 1 behind `HeapConfig::generational`: remembered lists, LOS cards, sticky marks, the verifier's S1 rules, GC-stress, a caught panic |
//! | `minor.rs` | S1 step 2: the minor GC (old cells stay, remembered hosts and dirty cards traced, weak parts and young-key ephemerons), the major GC, the policy, generational GC-stress, the generational invariant check, a caught panic |
//! | `shape.rs` | the shape region: ShapeId = cage offset, word0 edge, weak clearing, full region, shared commit limit, config |
//! | `verify.rs` | the verify after a GC (checked mark + checked sweep) finds each fault `Heap::verify` finds; the planted dangling slot; the E2 check |
//! | `v1.rs` | V1, the remembered-set verifier before every minor GC: when it runs, ages against colours, the card of each young reference of a large object; each negative control of 03-gc §7 for S1 (drop 1 in N barriers, old-only filter, elide after escape) fails V1, in a scenario and in the random mutator, and passes with the control off; the S1 forms of "no root rescan" and "no card rescan" (a minor GC that skips the remembered hosts or the dirty cards) fail V1 at the next minor GC |

use ep9_value::Value;

use crate::{CageConfig, CellKind, CellPtr, CellSize, Heap, HeapConfig, KindId, Slot, Tracer};

mod alloc;
mod barrier;
#[cfg(not(miri))]
mod css;
mod ephemeron;
mod gc;
mod minor;
mod model;
mod s1;
mod shape;
mod stress;
mod v1;
mod verify;

/// A test node: word0, a raw meta word `(id << 32) | n`, then `n` Value slots.
pub(crate) const NODE: KindId = KindId::new(10).unwrap();
/// A leaf: word0 and two raw words; no references. 24 bytes.
pub(crate) const LEAF: KindId = KindId::new(11).unwrap();
/// A weak box: word0, a weak slot at +8, a strong slot at +16.
pub(crate) const WEAKBOX: KindId = KindId::new(12).unwrap();

pub(crate) const NODE_HEADER: usize = 16;

fn node_slots(c: CellPtr) -> usize {
    // SAFETY: the heap calls size and trace functions only with live NODE cells, whose meta
    // word is written right after allocation.
    (unsafe { c.field::<u64>(8).read() } & 0xFFFF_FFFF) as usize
}

fn node_size(c: CellPtr) -> usize {
    NODE_HEADER + 8 * node_slots(c)
}

fn node_trace(c: CellPtr, t: &mut Tracer<'_>) {
    // SAFETY: a live NODE cell has `node_slots(c)` Value slots from offset 16.
    t.visit_slots(unsafe { c.slots(NODE_HEADER, node_slots(c)) });
}

fn leaf_trace(_: CellPtr, _: &mut Tracer<'_>) {}

fn weakbox_trace(c: CellPtr, t: &mut Tracer<'_>) {
    // SAFETY: a live WEAKBOX cell has Value slots at +8 and +16.
    unsafe {
        t.visit_weak(c.slot(8));
        t.visit(c.slot(16));
    }
}

pub(crate) fn register_test_kinds(heap: &Heap) {
    heap.register_kind(
        NODE,
        CellKind {
            name: "test-node",
            size: CellSize::Dynamic(node_size),
            trace: node_trace,
        },
    )
    .unwrap();
    heap.register_kind(
        LEAF,
        CellKind {
            name: "test-leaf",
            size: CellSize::Fixed(24),
            trace: leaf_trace,
        },
    )
    .unwrap();
    heap.register_kind(
        WEAKBOX,
        CellKind {
            name: "test-weakbox",
            size: CellSize::Fixed(24),
            trace: weakbox_trace,
        },
    )
    .unwrap();
}

/// A small test heap: a 1 GiB cage (8 MiB under Miri), zapping on, CSS roots recorded,
/// test kinds registered.
pub(crate) fn test_heap(tweak: impl FnOnce(&mut HeapConfig)) -> Heap {
    let mut cfg = HeapConfig {
        cage: CageConfig {
            cage_size: 1 << 30,
            code_range_size: 0,
            ..CageConfig::default()
        },
        zap: true,
        record_conservative_roots: true,
        ..HeapConfig::default()
    };
    tweak(&mut cfg);
    let heap = Heap::new(cfg).expect("test heap");
    register_test_kinds(&heap);
    heap
}

/// Allocates a node with id `id` and `n` slots (all `undefined`).
pub(crate) fn new_node(heap: &Heap, id: u32, n: usize) -> CellPtr {
    let c = heap.alloc(NODE, NODE_HEADER + 8 * n).expect("alloc node");
    // SAFETY: a new NODE cell; offset 8 is its meta word. Written before any GC point.
    unsafe { c.field::<u64>(8).write(((id as u64) << 32) | n as u64) };
    c
}

/// The id of a live node.
pub(crate) fn node_id(c: CellPtr) -> u32 {
    // SAFETY: callers pass live NODE cells.
    (unsafe { c.field::<u64>(8).read() } >> 32) as u32
}

/// Slot `i` of a live node.
pub(crate) fn slot(c: CellPtr, i: usize) -> &'static Slot {
    assert!(i < node_slots(c));
    // SAFETY: callers pass live NODE cells; slot i < n is a Value slot. The test keeps the
    // cell alive while it uses the reference.
    unsafe { c.slot(NODE_HEADER + 8 * i) }
}

/// Stores `v` into slot `i` of `host` through the barrier.
pub(crate) fn store(heap: &Heap, host: CellPtr, i: usize, v: Value) {
    // SAFETY: `host` is a live node (caller).
    slot(host, i).set(unsafe { host.header() }, v, heap.barrier());
}

/// Direct strong children of a live test cell (NODE slots, the WEAKBOX strong slot).
pub(crate) fn children(c: CellPtr) -> Vec<usize> {
    // SAFETY: callers pass live test cells.
    let kind = unsafe { c.header() }.kind();
    let vals: Vec<u64> = if kind == NODE.get() {
        (0..node_slots(c)).map(|i| slot(c, i).bits()).collect()
    } else if kind == WEAKBOX.get() {
        // SAFETY: a live WEAKBOX has a strong slot at +16.
        vec![unsafe { c.slot(16) }.bits()]
    } else {
        Vec::new()
    };
    vals.into_iter()
        .filter(|b| crate::barrier::is_cell_bits(*b))
        .map(|b| b as usize)
        .collect()
}

/// The cells that must be live after the last GC: everything reachable from `roots` and from
/// the cells the last conservative scan found. Panics if one of them is not a live cell.
pub(crate) fn expected_live(heap: &Heap, roots: &[usize]) -> std::collections::BTreeSet<usize> {
    let mut seen = std::collections::BTreeSet::new();
    let mut work: Vec<usize> = roots.to_vec();
    work.extend(heap.last_conservative_roots().iter().map(|c| c.addr()));
    while let Some(a) = work.pop() {
        if !seen.insert(a) {
            continue;
        }
        assert!(heap.is_cell(a), "cell {a:#x} must be live but was freed");
        // SAFETY: `a` is a live cell of this heap (checked above).
        let c = unsafe { CellPtr::from_raw(core::ptr::NonNull::new(heap_ptr(heap, a)).unwrap()) };
        work.extend(children(c));
    }
    seen
}

/// Writes `dead[0]` into every word of 16 KiB of stack below the caller's frame: the worst
/// stale words a garbage helper can leave in its dead frame. Call it right after the helper.
/// Then a test whose GC must find no live cell fails on every target, not only where two
/// frames happen to line up, if anything copies a stale stack word into the test's own frame
/// before [`clear_stack`]. Example: `heap.stats()` returns a `GcStats` whose 7 padding bytes
/// (after `last_minor`) come from the callee's stack as they were.
#[inline(never)]
pub(crate) fn paint_stack(dead: &[usize]) {
    let mut words = [dead[0]; 2048];
    std::hint::black_box(&mut words);
}

/// Zeroes 64 KiB of stack below the caller's frame. Stale words that earlier calls left there
/// (cell addresses in dead frames) then cannot keep a cell alive in the next conservative scan.
#[inline(never)]
pub(crate) fn clear_stack() {
    let mut words = [0usize; 8192];
    std::hint::black_box(&mut words);
}

/// A pointer to `addr` in the heap, with the reservation's provenance.
pub(crate) fn heap_ptr(heap: &Heap, addr: usize) -> *mut ep9_value::Word0 {
    assert!(heap.contains(addr));
    heap.inner.borrow().mem.ptr(addr).as_ptr().cast()
}
