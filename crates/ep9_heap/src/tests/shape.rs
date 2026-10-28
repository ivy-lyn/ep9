//! The shape region (H08-Q1): ShapeId = offset from the cage base, the word0 shape edge, weak
//! clearing, sweeping and reuse, the full region, config and space checks, GC-stress, and the
//! commit limit that both spaces share (sweep both queues before OutOfMemory).
//!
//! The heaps here have a 1 MiB region (4 chunks), which also fits the 8 MiB Miri reservation.
//! Under Miri the cage is aligned to 256 KiB only, so the id checks use the subtract.

use core::ptr::NonNull;

use ep9_value::{ShapeId, Value};

use super::*;
use crate::layout::{FIRST_CELL_OFFSET, MAX_SMALL_SIZE, PAGE_CELL_BYTES, PAGE_SIZE};
use crate::{AllocError, HeapError, KindError, Space, gc_state, set_shape};

/// A test shape: word0, a strong slot at +8 (like a shape's proto), a weak slot at +16 (like a
/// single transition). 24 bytes.
const SHAPE: KindId = KindId::new(20).unwrap();
/// A shape-space cell of 8 KiB with no references, to fill the region quickly.
const BIG_SHAPE: KindId = KindId::new(21).unwrap();
/// A shape-space kind whose size reads a length word at +8 (for the size limit).
const DYN_SHAPE: KindId = KindId::new(22).unwrap();
/// Region size of the test heaps.
const REGION: usize = 1 << 20;

fn shape_trace(c: CellPtr, t: &mut Tracer<'_>) {
    // SAFETY: a live SHAPE cell has Value slots at +8 and +16.
    unsafe {
        t.visit(c.slot(8));
        t.visit_weak(c.slot(16));
    }
}

fn no_trace(_: CellPtr, _: &mut Tracer<'_>) {}

fn dyn_size(c: CellPtr) -> usize {
    // SAFETY: a live DYN_SHAPE cell keeps its size at +8, written right after allocation.
    unsafe { c.field::<u64>(8).read() as usize }
}

fn shape_heap(tweak: impl FnOnce(&mut HeapConfig)) -> Heap {
    let heap = test_heap(|c| {
        c.shape_space_bytes = REGION;
        tweak(c);
    });
    let kind = |name, size| CellKind {
        name,
        size,
        trace: no_trace,
    };
    heap.register_kind_in(
        SHAPE,
        CellKind {
            trace: shape_trace,
            ..kind("test-shape", CellSize::Fixed(24))
        },
        Space::Shape,
    )
    .unwrap();
    heap.register_kind_in(
        BIG_SHAPE,
        kind("test-big-shape", CellSize::Fixed(MAX_SMALL_SIZE)),
        Space::Shape,
    )
    .unwrap();
    heap.register_kind_in(
        DYN_SHAPE,
        kind("test-dyn-shape", CellSize::Dynamic(dyn_size)),
        Space::Shape,
    )
    .unwrap();
    heap
}

fn new_shape(heap: &Heap) -> CellPtr {
    heap.alloc_shape(SHAPE, 24).expect("alloc shape")
}

fn in_region(heap: &Heap, addr: usize) -> bool {
    addr.wrapping_sub(heap.cage_base()) < heap.shape_space_bytes()
}

/// Installs `shape` (or ShapeId 0) in `host`'s word0 with the shape-install barrier.
fn install(heap: &Heap, host: CellPtr, shape: Option<CellPtr>) {
    let id = shape.map_or(ShapeId::new(0), |s| heap.shape_id_of(s));
    // SAFETY: `host` is live (caller).
    set_shape(unsafe { host.header() }, id, heap.barrier());
}

/// A full collection with precise roots only.
fn precise_gc(heap: &Heap) {
    heap.collect_with_words(&[]);
    assert!(heap.last_conservative_roots().is_empty());
}

/// A CellPtr for `addr` while a live cell starts there.
fn cell_at(heap: &Heap, addr: usize) -> CellPtr {
    assert!(heap.is_cell(addr), "{addr:#x} is not a live cell");
    // SAFETY: a live cell starts at `addr` (checked).
    unsafe { CellPtr::from_raw(NonNull::new(heap_ptr(heap, addr)).unwrap()) }
}

/// Slot `off` of a live cell.
fn field(c: CellPtr, off: usize) -> &'static Slot {
    // SAFETY: callers pass a live cell with a Value slot at `off` and keep it alive while they
    // use the reference.
    unsafe { c.slot(off) }
}

/// Stores `v` into slot `off` of `host` through the barrier.
fn set_field(heap: &Heap, host: CellPtr, off: usize, v: Value) {
    // SAFETY: `host` is live (caller).
    field(host, off).set(unsafe { host.header() }, v, heap.barrier());
}

#[test]
fn shape_id_is_the_offset_from_the_cage_base() {
    let heap = shape_heap(|_| {});
    assert_eq!(heap.shape_space_bytes(), REGION);
    let base = heap.cage_base();
    let s = new_shape(&heap);
    let id = heap.shape_id_of(s);
    assert_eq!(id.get() as usize, s.addr() - base, "id = addr - cage base");
    assert!(in_region(&heap, s.addr()));
    // The region is chunks [0, N): the first shape is the first cell of chunk 0.
    assert_eq!(id.get() as usize, FIRST_CELL_OFFSET);
    // SAFETY: `s` is live (a local; no GC point since).
    assert_eq!(unsafe { heap.shape_cell(id) }, s);
    #[cfg(not(miri))]
    {
        assert!(base.is_multiple_of(1 << 32), "the cage is 4 GiB-aligned");
        assert_eq!(id.get(), s.addr() as u32, "id = low 32 bits of the address");
    }
    // JIT and AOT code read the base at a fixed offset.
    // SAFETY: offset 24 of the heap is the `usize` cage base (asserted at compile time); it
    // never changes.
    let raw = unsafe {
        core::ptr::from_ref(&heap)
            .cast::<u8>()
            .add(Heap::CAGE_BASE_OFFSET)
            .cast::<usize>()
            .read()
    };
    assert_eq!(raw, base);
    // Offset 0 never holds a cell, so id 0 means "none".
    assert!(!heap.is_cell(base));
    assert_eq!(heap.find_cell(base), None);
    // A new shape is like any new cell, and interior pointers find it (CSS).
    // SAFETY: `s` is live.
    let h = unsafe { s.header() };
    assert_eq!(
        (h.shape_id(), h.gc_state(), h.kind()),
        (ShapeId::new(0), gc_state::WHITE, SHAPE.get())
    );
    assert!(field(s, 8).get().is_undefined() && field(s, 16).get().is_undefined());
    assert_eq!(heap.find_cell(s.addr() + 17), Some(s));
    // Ordinary cells, small and large, never land in the region.
    let n = new_node(&heap, 1, 2);
    let big = new_node(&heap, 2, 2000);
    assert!(n.addr() >= base + REGION && big.addr() >= base + REGION);
    let s2 = new_shape(&heap);
    assert_eq!(s2.addr(), s.addr() + 24, "shapes bump in their own hole");
    let st = heap.stats();
    assert_eq!((st.shape_pages, st.small_pages, st.large_cells), (1, 2, 1));
    heap.verify().expect("verifier");
}

/// Allocates a shape whose strong slot holds a leaf, installs it in `obj`'s word0, and
/// returns both addresses as integers (which no scan reads).
#[inline(never)]
fn shape_with_leaf(heap: &Heap, obj: CellPtr) -> (usize, usize) {
    let roots = heap.scratch_roots();
    let s = new_shape(heap);
    roots.push(s.to_value());
    let leaf = heap.alloc(LEAF, 24).expect("leaf");
    let s = CellPtr::from_value(roots.get(0)).unwrap();
    set_field(heap, s, 8, leaf.to_value());
    install(heap, obj, Some(s));
    (s.addr(), leaf.addr())
}

#[test]
fn a_shape_referenced_only_from_word0_survives() {
    let heap = shape_heap(|_| {});
    let obj = new_node(&heap, 1, 0);
    let g = heap.new_global(obj.to_value());
    let (s, leaf) = shape_with_leaf(&heap, obj);
    precise_gc(&heap);
    assert!(heap.is_cell(s), "the shape that word0 names survives");
    assert!(heap.is_cell(leaf), "and the GC traces it");
    // SAFETY: `obj` is rooted by `g`.
    let id = unsafe { obj.header() }.shape_id();
    assert_eq!(id.get() as usize, s - heap.cage_base());
    // SAFETY: the shape is live.
    assert_eq!(unsafe { heap.shape_cell(id) }.addr(), s);
    heap.verify().expect("verifier");
    // A second GC: the shape is now BLACK and still kept.
    precise_gc(&heap);
    assert!(heap.is_cell(s) && heap.is_cell(leaf));
    // ShapeId 0 is no edge: the shape and its leaf die.
    install(&heap, obj, None);
    precise_gc(&heap);
    assert!(!heap.is_cell(s) && !heap.is_cell(leaf));
    heap.drop_global(g);
    heap.verify().expect("verifier");
}

/// A weak box rooted by a handle whose weak slot holds a new shape. Returns the box and the
/// shape's address.
#[inline(never)]
fn weak_box_to_new_shape(heap: &Heap) -> (crate::GlobalHandle, usize) {
    let wb = heap.alloc(WEAKBOX, 24).expect("weak box");
    let g = heap.new_global(wb.to_value());
    let s = new_shape(heap);
    set_field(heap, wb, 8, s.to_value());
    (g, s.addr())
}

#[test]
fn a_dead_shape_is_freed_and_weak_slots_to_it_cleared_in_the_same_gc() {
    let heap = shape_heap(|_| {});
    // Weak box -> shape D (no other reference): D dies.
    let (g_dead, d) = weak_box_to_new_shape(&heap);
    // Weak box -> shape K, which an object's word0 also names: K lives.
    let (g_kept, k) = weak_box_to_new_shape(&heap);
    let obj = new_node(&heap, 1, 0);
    let g_obj = heap.new_global(obj.to_value());
    install(&heap, obj, Some(cell_at(&heap, k)));
    // Shape K's weak slot (a transition) -> shape T (no other reference): T dies.
    let t = new_shape(&heap).addr();
    set_field(&heap, cell_at(&heap, k), 16, cell_at(&heap, t).to_value());
    let before = heap.stats().collections;
    precise_gc(&heap);
    assert_eq!(heap.stats().collections, before + 1);
    assert!(!heap.live_after_gc(d) && !heap.live_after_gc(t));
    assert!(heap.live_after_gc(k));
    let wb_dead = CellPtr::from_value(heap.global(g_dead)).unwrap();
    let wb_kept = CellPtr::from_value(heap.global(g_kept)).unwrap();
    assert!(
        field(wb_dead, 8).get().is_undefined(),
        "cleared in the GC that freed D"
    );
    assert_eq!(
        field(wb_kept, 8).bits(),
        cell_at(&heap, k).to_value().to_bits()
    );
    assert!(
        field(cell_at(&heap, k), 16).get().is_undefined(),
        "dead transition cleared"
    );
    assert_eq!(heap.stats().last_weak_cleared, 2);
    assert!(!heap.is_cell(d) && !heap.is_cell(t));
    for g in [g_dead, g_kept, g_obj] {
        heap.drop_global(g);
    }
    heap.verify().expect("verifier");
}

#[test]
fn a_full_region_gives_out_of_memory_after_one_gc() {
    let heap = shape_heap(|_| {});
    let per_page = PAGE_CELL_BYTES / MAX_SMALL_SIZE;
    let pages = REGION / PAGE_SIZE;
    let roots = heap.scratch_roots();
    loop {
        let before = heap.stats().collections;
        match heap.alloc_shape(BIG_SHAPE, MAX_SMALL_SIZE) {
            Ok(c) => {
                assert!(in_region(&heap, c.addr()));
                roots.push(c.to_value());
                assert_eq!(heap.stats().collections, before);
            }
            Err(e) => {
                assert_eq!(e, AllocError::OutOfMemory);
                assert_eq!(heap.stats().collections, before + 1, "exactly one GC");
                break;
            }
        }
    }
    assert_eq!(roots.len(), per_page * pages);
    assert_eq!(heap.stats().shape_pages, pages);
    // Ordinary allocation does not use the region and still works.
    let n = new_node(&heap, 1, 4);
    assert!(!in_region(&heap, n.addr()));
    // A small shape still fits the tail hole that each full page keeps.
    let s = new_shape(&heap);
    let tail = (s.addr() - heap.cage_base()) % PAGE_SIZE;
    assert!(tail >= FIRST_CELL_OFFSET + per_page * MAX_SMALL_SIZE);
    // With the roots gone, the next big shape gets a page back after one GC.
    drop(roots);
    let before = heap.stats().collections;
    let c = heap
        .alloc_shape(BIG_SHAPE, MAX_SMALL_SIZE)
        .expect("space after a GC");
    assert!(in_region(&heap, c.addr()));
    assert_eq!(heap.stats().collections, before + 1);
    heap.verify().expect("verifier");
}

/// Allocates `n` unrooted big shapes and returns their addresses.
#[inline(never)]
fn big_shape_garbage(heap: &Heap, n: usize) -> Vec<usize> {
    (0..n)
        .map(|_| heap.alloc_shape(BIG_SHAPE, MAX_SMALL_SIZE).unwrap().addr())
        .collect()
}

#[test]
fn empty_shape_pages_go_back_to_the_region() {
    let heap = shape_heap(|_| {});
    let per_page = PAGE_CELL_BYTES / MAX_SMALL_SIZE;
    let dead = big_shape_garbage(&heap, per_page + 1);
    let st = heap.stats();
    assert_eq!(st.shape_pages, 2);
    let committed = st.committed_bytes;
    precise_gc(&heap);
    assert_eq!(heap.unswept_pages(), 2, "shape pages are swept lazily");
    heap.finish_sweeping();
    for d in dead {
        assert!(!heap.is_cell(d));
    }
    let st = heap.stats();
    assert_eq!(st.shape_pages, 0);
    assert_eq!(committed - st.committed_bytes, 2 * PAGE_SIZE, "decommitted");
    // The lowest region chunk is reused first; ordinary pages still never use it.
    let s = new_shape(&heap);
    assert_eq!((s.addr() - heap.cage_base()) / PAGE_SIZE, 0);
    assert!(!in_region(&heap, new_node(&heap, 1, 1).addr()));
    heap.verify().expect("verifier");
}

#[test]
fn a_shape_hole_is_reused_after_a_sweep() {
    let heap = shape_heap(|_| {});
    let a = new_shape(&heap);
    let g = heap.new_global(a.to_value());
    let dead: Vec<usize> = big_shape_garbage(&heap, 2);
    let c = new_shape(&heap);
    let g2 = heap.new_global(c.to_value());
    precise_gc(&heap);
    // The page is recyclable: the next shapes fill the free lines where the big ones were.
    let s = new_shape(&heap);
    assert!(
        s.addr() > a.addr() && s.addr() < c.addr(),
        "{s:?} between {a:?} and {c:?}"
    );
    assert!(dead.iter().all(|&d| !heap.is_cell(d)));
    assert_eq!(heap.stats().shape_pages, 1);
    heap.drop_global(g);
    heap.drop_global(g2);
    heap.verify().expect("verifier");
}

#[cfg(not(miri))]
#[test]
fn the_conservative_scan_keeps_a_shape() {
    let heap = shape_heap(|_| {});
    let s = new_shape(&heap).addr();
    // An interior pointer, as a stack word would hold it.
    heap.collect_with_words(&[s + 8]);
    assert!(heap.is_cell(s));
    assert_eq!(
        heap.last_conservative_roots()
            .iter()
            .map(|c| c.addr())
            .collect::<Vec<_>>(),
        vec![s]
    );
    heap.collect_with_words(&[]);
    assert!(!heap.is_cell(s));
}

fn config(shape_space_bytes: usize, tweak: impl FnOnce(&mut HeapConfig)) -> HeapConfig {
    let mut c = HeapConfig {
        cage: CageConfig {
            cage_size: 1 << 30,
            code_range_size: 0,
            ..CageConfig::default()
        },
        shape_space_bytes,
        ..HeapConfig::default()
    };
    tweak(&mut c);
    c
}

fn config_error(c: HeapConfig) -> &'static str {
    match Heap::new(c) {
        Err(HeapError::Config(m)) => m,
        Err(e) => panic!("not a config error: {e}"),
        Ok(_) => panic!("config accepted"),
    }
}

#[test]
fn shape_space_config_is_checked() {
    assert_eq!(HeapConfig::default().shape_space_bytes, 0, "off by default");
    let h = Heap::new(config(0, |_| {})).unwrap();
    assert_eq!((h.shape_space_bytes(), h.stats().shape_pages), (0, 0));
    drop(h);
    for bad in [3 << 20, 512 << 10, 8 << 30, (1 << 20) + PAGE_SIZE] {
        assert!(
            config_error(config(bad, |_| {})).contains("power of two"),
            "{bad:#x}"
        );
    }
    assert!(config_error(config(2 << 30, |_| {})).contains("larger than the cage"));
    let ok = Heap::new(config(1 << 20, |_| {})).unwrap();
    assert_eq!(ok.shape_space_bytes(), 1 << 20);
    drop(ok);
    #[cfg(not(miri))]
    {
        // align < 4 GiB with a shape space is rejected; without one it is fine.
        let small_align = |c: &mut HeapConfig| c.cage.align = 1 << 30;
        assert!(config_error(config(1 << 20, small_align)).contains("cage.align"));
        assert!(Heap::new(config(0, small_align)).is_ok());
        // The largest region: 4 GiB, in the default 64 GiB cage. Its ids still fit a u32.
        let heap = Heap::new(HeapConfig {
            shape_space_bytes: 4 << 30,
            ..HeapConfig::default()
        })
        .unwrap();
        assert_eq!(heap.shape_space_bytes(), 4 << 30);
        heap.register_kind_in(
            SHAPE,
            CellKind {
                name: "test-shape",
                size: CellSize::Fixed(24),
                trace: shape_trace,
            },
            Space::Shape,
        )
        .unwrap();
        let s = heap.alloc_shape(SHAPE, 24).unwrap();
        assert_eq!(heap.shape_id_of(s).get(), s.addr() as u32);
        heap.verify().expect("verifier");
    }
    #[cfg(miri)]
    {
        // Miri reserves 8 MiB: a bigger region does not fit.
        assert!(config_error(config(16 << 20, |_| {})).contains("reservation"));
    }
}

#[test]
fn kinds_belong_to_one_space() {
    let shape_kind = CellKind {
        name: "test-shape",
        size: CellSize::Fixed(24),
        trace: shape_trace,
    };
    // No shape space: a shape kind cannot be registered, an ordinary one can.
    let plain = test_heap(|_| {});
    assert_eq!(
        plain.register_kind_in(SHAPE, shape_kind, Space::Shape),
        Err(KindError::NoShapeSpace(SHAPE))
    );
    assert_eq!(
        plain.alloc_shape(SHAPE, 24).unwrap_err(),
        AllocError::UnknownKind(SHAPE)
    );
    assert_eq!(
        plain.alloc_shape(NODE, 16).unwrap_err(),
        AllocError::WrongSpace(NODE)
    );
    drop(plain);

    let heap = shape_heap(|_| {});
    assert_eq!(
        heap.register_kind_in(SHAPE, shape_kind, Space::Shape),
        Err(KindError::Taken(SHAPE))
    );
    let too_big = KindId::new(23).unwrap();
    assert_eq!(
        heap.register_kind_in(
            too_big,
            CellKind {
                size: CellSize::Fixed(MAX_SMALL_SIZE + 8),
                ..shape_kind
            },
            Space::Shape
        ),
        Err(KindError::BadSize(too_big))
    );
    // The same size is fine for an ordinary kind (it goes to LOS).
    heap.register_kind_in(
        too_big,
        CellKind {
            size: CellSize::Fixed(MAX_SMALL_SIZE + 8),
            ..shape_kind
        },
        Space::Normal,
    )
    .unwrap();
    // Each entry takes only its own space's kinds. `alloc` keeps its code as it was, so a
    // shape kind is simply not in its table.
    assert_eq!(
        heap.alloc(SHAPE, 24).unwrap_err(),
        AllocError::UnknownKind(SHAPE)
    );
    assert_eq!(
        heap.alloc_shape(LEAF, 24).unwrap_err(),
        AllocError::WrongSpace(LEAF)
    );
    let none = KindId::new(99).unwrap();
    assert_eq!(
        heap.alloc(none, 16).unwrap_err(),
        AllocError::UnknownKind(none)
    );
    assert_eq!(
        heap.alloc_shape(none, 16).unwrap_err(),
        AllocError::UnknownKind(none)
    );
    assert_eq!(
        heap.alloc_shape(SHAPE, 32).unwrap_err(),
        AllocError::WrongSize(SHAPE)
    );
    assert_eq!(
        heap.alloc_shape(DYN_SHAPE, MAX_SMALL_SIZE + 8).unwrap_err(),
        AllocError::TooLarge
    );
    assert_eq!(
        heap.alloc_shape(DYN_SHAPE, usize::MAX).unwrap_err(),
        AllocError::TooLarge
    );
    let d = heap.alloc_shape(DYN_SHAPE, 40).unwrap();
    // SAFETY: a new DYN_SHAPE cell; +8 is its size word. Written before any GC point.
    unsafe { d.field::<u64>(8).write(40) };
    assert!(in_region(&heap, d.addr()));
    assert!(
        AllocError::WrongSpace(LEAF)
            .to_string()
            .contains("not a shape-space kind")
    );
    assert!(
        KindError::NoShapeSpace(SHAPE)
            .to_string()
            .contains("shape space")
    );
    heap.verify().expect("verifier");
}

#[test]
fn the_verifier_rejects_a_word0_shape_that_names_no_shape() {
    let heap = shape_heap(|_| {});
    let obj = new_node(&heap, 1, 1);
    let s = new_shape(&heap);
    let g = heap.new_global(obj.to_value());
    // Slot 0 keeps `s` alive, so the bad-id cases below do not free it.
    store(&heap, obj, 0, s.to_value());
    let id = heap.shape_id_of(s).get();
    // SAFETY: `obj` is rooted.
    let h = unsafe { obj.header() };
    for bad in [
        id + 8,                                                   // inside a shape, not its start
        (REGION - 16) as u32,                                     // free region memory
        (new_node(&heap, 2, 0).addr() - heap.cage_base()) as u32, // an ordinary cell
    ] {
        h.set_shape_id(ShapeId::new(bad));
        let e = heap.verify().unwrap_err().to_string();
        assert!(e.contains("ShapeId"), "{bad:#x}: {e}");
    }
    h.set_shape_id(ShapeId::new(id));
    heap.verify().expect("a valid id passes");
    heap.drop_global(g);
}

/// Objects with shapes, shapes with strong children and weak transitions, under GC-stress 1
/// with the verifier after every GC: the verifier checks every word0 ShapeId each time.
#[test]
fn shapes_under_gc_stress() {
    let iters: u32 = if cfg!(miri) { 24 } else { 1500 };
    const LIVE: usize = 8;
    let heap = shape_heap(|c| {
        c.gc_stress = 1;
        c.verify_after_gc = true;
    });
    let root = new_node(&heap, 0, LIVE);
    let g = heap.new_global(root.to_value());
    for i in 0..iters {
        let roots = heap.scratch_roots();
        let s = new_shape(&heap);
        roots.push(s.to_value());
        let leaf = heap.alloc(LEAF, 24).expect("leaf");
        let s = CellPtr::from_value(roots.get(0)).unwrap();
        set_field(&heap, s, 8, leaf.to_value());
        // Weak transition to the shape of the previous object, if it is still there.
        let prev = slot(root, (i as usize + LIVE - 1) % LIVE).get();
        if let Some(p) = CellPtr::from_value(prev) {
            // SAFETY: `p` is rooted through `root`.
            let id = unsafe { p.header() }.shape_id();
            // SAFETY: a live object's word0 names a live shape.
            let ps = unsafe { heap.shape_cell(id) };
            set_field(&heap, s, 16, ps.to_value());
        }
        let obj = new_node(&heap, i, 0);
        let s = CellPtr::from_value(roots.get(0)).unwrap();
        install(&heap, obj, Some(s));
        // Replaces an older object: its shape dies unless a word0 still names it.
        store(&heap, root, i as usize % LIVE, obj.to_value());
    }
    precise_gc(&heap);
    heap.finish_sweeping();
    let mut shapes = Vec::new();
    for k in 0..LIVE {
        let obj = CellPtr::from_value(slot(root, k).get()).unwrap();
        // SAFETY: rooted through `root`.
        let id = unsafe { obj.header() }.shape_id();
        assert_ne!(id.get(), 0);
        let s = id.get() as usize + heap.cage_base();
        assert!(heap.is_cell(s), "object {k}: its shape is live");
        let leaf = field(cell_at(&heap, s), 8).get();
        assert!(heap.is_cell(CellPtr::from_value(leaf).unwrap().addr()));
        shapes.push(s);
    }
    // Only these shapes survive: the region holds exactly LIVE shape cells.
    let r = heap.verify().expect("verifier");
    let st = heap.stats();
    assert!(st.collections > u64::from(iters));
    assert!(st.shape_pages >= 1 && r.small_cells > 3 * LIVE);
    assert_eq!(heap.shape_region_cells(), LIVE, "{shapes:x?}");
    heap.drop_global(g);
}

/// One unrooted leaf in a fresh ordinary page, allocated in a frame of its own. Returns its
/// address in malloc memory, which the conservative scan never reads.
#[inline(never)]
fn ordinary_garbage(heap: &Heap) -> Vec<usize> {
    vec![heap.alloc(LEAF, 24).expect("leaf").addr()]
}

/// One unrooted shape in a fresh shape page, as [`ordinary_garbage`].
#[inline(never)]
fn shape_garbage(heap: &Heap) -> Vec<usize> {
    vec![new_shape(heap).addr()]
}

/// The two spaces share `max_heap_bytes` (one page here). A shape page needs the commit room
/// of an ordinary page that dies in the shape allocation's own GC. That page waits in the
/// ordinary lazy sweep queue, so the allocation must sweep it before it gives OutOfMemory.
/// Verifier off: it finishes the sweep and would hide the failure.
#[test]
fn alloc_shape_sweeps_dead_ordinary_pages_before_out_of_memory() {
    let heap = shape_heap(|c| c.max_heap_bytes = PAGE_SIZE);
    let dead = ordinary_garbage(&heap);
    paint_stack(&dead);
    clear_stack();
    assert_eq!(heap.stats().committed_bytes, PAGE_SIZE);
    let before = heap.stats().collections;
    let s = heap.alloc_shape(SHAPE, 24).map(|c| c.addr());
    let st = heap.stats();
    assert_eq!(st.last_marked_cells, 0, "the GC found no live cell");
    assert_eq!(st.collections, before + 1, "one GC");
    let s = s.expect("the dead ordinary page makes room");
    assert!(in_region(&heap, s));
    assert_eq!((st.small_pages, st.shape_pages), (1, 1));
    assert_eq!(st.committed_bytes, PAGE_SIZE);
    assert!(!heap.is_cell(dead[0]) && heap.is_cell(s));
    heap.verify().expect("verifier");
}

/// The other direction: an ordinary page needs the commit room of a shape page that dies in
/// the allocation's own GC and waits in the shape sweep queue.
#[test]
fn alloc_sweeps_dead_shape_pages_before_out_of_memory() {
    let heap = shape_heap(|c| c.max_heap_bytes = PAGE_SIZE);
    let dead = shape_garbage(&heap);
    paint_stack(&dead);
    clear_stack();
    assert_eq!(heap.stats().committed_bytes, PAGE_SIZE);
    let before = heap.stats().collections;
    let n = heap.alloc(LEAF, 24).map(|c| c.addr());
    let st = heap.stats();
    assert_eq!(st.last_marked_cells, 0, "the GC found no live cell");
    assert_eq!(st.collections, before + 1, "one GC");
    let n = n.expect("the dead shape page makes room");
    assert!(!in_region(&heap, n));
    assert_eq!((st.small_pages, st.shape_pages), (1, 0));
    assert_eq!(st.committed_bytes, PAGE_SIZE);
    assert!(!heap.is_cell(dead[0]) && heap.is_cell(n));
    heap.verify().expect("verifier");
}

/// A large cell (LOS) needs the commit room of a shape page that dies in its own GC.
#[test]
fn a_large_cell_sweeps_dead_shape_pages_before_out_of_memory() {
    let heap = shape_heap(|c| c.max_heap_bytes = PAGE_SIZE);
    let dead = shape_garbage(&heap);
    paint_stack(&dead);
    clear_stack();
    let before = heap.stats().collections;
    let big = heap.alloc(NODE, NODE_HEADER + 8 * 2000);
    if let Ok(c) = big {
        // SAFETY: a new NODE cell; meta word written before the next GC point.
        unsafe { c.field::<u64>(8).write(2000) };
    }
    let st = heap.stats();
    assert_eq!(st.last_marked_cells, 0, "the GC found no live cell");
    assert_eq!(st.collections, before + 1, "one GC");
    let big = big.expect("the dead shape page makes room");
    assert_eq!((st.small_pages, st.large_cells), (0, 1));
    assert!(st.committed_bytes <= PAGE_SIZE);
    assert!(!heap.is_cell(dead[0]) && heap.is_cell(big.addr()));
    heap.verify().expect("verifier");
}
