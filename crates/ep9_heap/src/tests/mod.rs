//! Tests of the whole heap. Per-module unit tests live next to their module.
//!
//! | File | Covers |
//! |---|---|
//! | `barrier.rs` | barrier v2: filter, threshold compare, slow path stores, shape install, init |
//! | `model.rs` | the H11 decision model: exhaustive interleavings, 6 scenarios, negative controls |
//! | `alloc.rs` | TLAB, holes, overflow, LOS, pre-fill, kinds, limits, offsets |
//! | `gc.rs` | mark-sweep with precise roots, weak slots, lazy sweep, zapping, GC-stress, verifier |
//! | `css.rs` | conservative stack scan: locals, interior and one-past-end pointers, S7 (not Miri) |
//! | `stress.rs` | randomized mutator against a model of the object graph |
//! | `shape.rs` | the shape region: ShapeId = cage offset, word0 edge, weak clearing, full region, shared commit limit, config |

use ep9_value::Value;

use crate::{CageConfig, CellKind, CellPtr, CellSize, Heap, HeapConfig, KindId, Slot, Tracer};

mod alloc;
mod barrier;
#[cfg(not(miri))]
mod css;
mod gc;
mod model;
mod shape;
mod stress;

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
            code_range_size: ep9_platform::allocation_granularity(),
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
