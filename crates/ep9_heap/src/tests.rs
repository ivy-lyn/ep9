//! Barrier prototype tests: when the slow path runs, and that every store lands.

use core::cell::Cell;
use core::ptr::NonNull;

use ep9_value::encoding::CELL_ADDRESS_LIMIT;
use ep9_value::{ShapeId, Value, Word0};

use crate::barrier::probe::{ep9_probe_barrier, ep9_probe_barrier_new, ep9_probe_barrier_old};
use crate::barrier::{BarrierState, Slot, is_cell_bits};

/// Fake cells: 8-aligned headers in memory below 2^47 by construction (H7). The barrier only
/// reads their gc_state byte. A `Box` is not enough: on Linux arm64 the malloc heap and PIE
/// images sit above 2^47 (0xaaaa_..., 0xffff_...).
/// - Native: one committed page of a small cage from `ep9_platform`.
/// - Miri: Miri cannot map a fixed address range, but it gives its own allocations low
///   addresses, so a plain allocation is below 2^47 there.
struct FakeHeap {
    #[cfg(not(miri))]
    _cage: ep9_platform::Cage,
    base: NonNull<Word0>,
    cap: usize,
    used: Cell<usize>,
}

/// Cells in the Miri buffer. The native page holds at least 512.
#[cfg(miri)]
const MIRI_CELLS: usize = 8;

#[cfg(miri)]
fn miri_layout() -> std::alloc::Layout {
    std::alloc::Layout::array::<Word0>(MIRI_CELLS).expect("layout")
}

impl FakeHeap {
    #[cfg(not(miri))]
    fn new() -> FakeHeap {
        use ep9_platform::{Cage, CageConfig, allocation_granularity, page_size};
        let g = allocation_granularity();
        let config = CageConfig {
            cage_size: g,
            code_range_size: g,
            ..CageConfig::default()
        };
        let cage = Cage::reserve(&config).expect("reserve a small cage below 2^47");
        cage.heap().commit(0, page_size()).expect("commit one page");
        // The cage got its base from mmap/VirtualAlloc as an exposed address.
        let base = core::ptr::with_exposed_provenance_mut::<Word0>(cage.heap().base());
        FakeHeap {
            _cage: cage,
            base: NonNull::new(base).expect("cage base is not null"),
            cap: page_size() / size_of::<Word0>(),
            used: Cell::new(0),
        }
    }

    #[cfg(miri)]
    fn new() -> FakeHeap {
        // SAFETY: the layout has a non-zero size.
        let p = unsafe { std::alloc::alloc_zeroed(miri_layout()) };
        FakeHeap {
            base: NonNull::new(p.cast()).expect("allocation failed"),
            cap: MIRI_CELLS,
            used: Cell::new(0),
        }
    }

    /// A new fake cell with this gc_state.
    fn cell(&self, gc_state: u8) -> &Word0 {
        let i = self.used.get();
        assert!(i < self.cap, "fake heap is full");
        self.used.set(i + 1);
        // SAFETY: cell `i` is inside the committed memory (checked above) and 8-aligned, and no
        // earlier call handed it out, so nothing else refers to it. The reference borrows
        // `self`, which keeps the memory mapped.
        unsafe {
            let p = self.base.add(i);
            p.write(Word0::new(ShapeId::new(8), gc_state, 0, 0));
            p.as_ref()
        }
    }
}

#[cfg(miri)]
impl Drop for FakeHeap {
    fn drop(&mut self) {
        // SAFETY: `base` came from `alloc_zeroed(miri_layout())` in `new` and is freed only here.
        // Word0 has no destructor, so the cells need no drop.
        unsafe { std::alloc::dealloc(self.base.as_ptr().cast(), miri_layout()) }
    }
}

fn cell_value(c: &Word0) -> Value {
    let addr = core::ptr::from_ref(c).addr() as u64;
    assert!(
        addr < CELL_ADDRESS_LIMIT,
        "fake cell address {addr:#x} above 2^47"
    );
    // SAFETY: `c` is a live, 8-aligned fake cell below 2^47 (checked above). It stands in for a
    // GC cell, and each test keeps its FakeHeap alive while it uses the Value.
    unsafe { Value::from_cell_ptr(NonNull::from(c)) }
}

fn non_cells() -> [Value; 7] {
    [
        Value::int32(0),
        Value::int32(-1),
        Value::double(0.5),
        Value::double(f64::NAN),
        Value::UNDEFINED,
        Value::NULL,
        Value::EMPTY,
    ]
}

type Store = extern "C" fn(&Word0, &Slot, Value, &BarrierState);

/// Runs one store and returns whether it took the slow path. Checks the store landed.
fn slow_taken(store: Store, host: &Word0, old: Value, new: Value, st: &BarrierState) -> bool {
    let slot = Slot::new(old);
    let before = st.slow_calls();
    store(host, &slot, new, st);
    assert_eq!(slot.get().to_bits(), new.to_bits(), "store must land");
    st.slow_calls() - before == 1
}

#[test]
fn barrier_off_never_takes_the_slow_path() {
    let st = BarrierState::new(BarrierState::OFF);
    let heap = FakeHeap::new();
    let target = cell_value(heap.cell(1));
    for state in 1..=u8::MAX {
        let host = Word0::new(ShapeId::new(8), state, 0, 0);
        for v in non_cells().into_iter().chain([target]) {
            for store in [
                ep9_probe_barrier as Store,
                ep9_probe_barrier_new,
                ep9_probe_barrier_old,
            ] {
                assert!(!slow_taken(store, &host, target, v, &st));
                assert!(!slow_taken(store, &host, v, target, &st));
            }
        }
    }
    assert_eq!(st.slow_calls(), 0);
}

#[test]
fn slot_set_filters_on_old_or_new() {
    let st = BarrierState::new(1);
    let heap = FakeHeap::new();
    let host = heap.cell(1); // gc_state <= threshold: barrier armed for this host
    let quiet = heap.cell(2); // gc_state > threshold: never barriered
    let cell = cell_value(heap.cell(3));
    for nc in non_cells() {
        // old or new is a cell -> slow path
        assert!(slow_taken(ep9_probe_barrier, host, nc, cell, &st)); // new cell (generations)
        assert!(slow_taken(ep9_probe_barrier, host, cell, nc, &st)); // old cell (SATB)
        assert!(slow_taken(ep9_probe_barrier, host, cell, cell, &st));
        // no cell -> fast path
        for nc2 in non_cells() {
            assert!(!slow_taken(ep9_probe_barrier, host, nc, nc2, &st));
        }
        // host above the threshold -> fast path whatever the values
        assert!(!slow_taken(ep9_probe_barrier, quiet, nc, cell, &st));
        assert!(!slow_taken(ep9_probe_barrier, quiet, cell, nc, &st));
    }
}

#[test]
fn single_value_forms_miss_one_case_each() {
    let st = BarrierState::new(1);
    let heap = FakeHeap::new();
    let host = heap.cell(1);
    let cell = cell_value(heap.cell(3));
    let int = Value::int32(5);
    // H11 text (new only): `host.f = 5` over a cell is missed -> SATB would lose the old cell.
    assert!(!slow_taken(ep9_probe_barrier_new, host, cell, int, &st));
    assert!(slow_taken(ep9_probe_barrier_new, host, int, cell, &st));
    // Old only: `host.f = cell` over an int is missed -> generations would miss the new ref.
    assert!(!slow_taken(ep9_probe_barrier_old, host, int, cell, &st));
    assert!(slow_taken(ep9_probe_barrier_old, host, cell, int, &st));
    // Slot::set takes both.
    assert!(slow_taken(ep9_probe_barrier, host, cell, int, &st));
    assert!(slow_taken(ep9_probe_barrier, host, int, cell, &st));
}

#[test]
fn threshold_compare_is_unsigned_and_inclusive() {
    let heap = FakeHeap::new();
    let cell = cell_value(heap.cell(3));
    for threshold in [1u8, 7, 0xFE, 0xFF] {
        let st = BarrierState::new(threshold);
        for state in [1u8, threshold, threshold.saturating_add(1), 0xFF] {
            let host = Word0::new(ShapeId::new(8), state, 0, 0);
            let want = state <= threshold;
            assert_eq!(slow_taken(ep9_probe_barrier, &host, cell, cell, &st), want);
        }
    }
}

#[test]
fn slot_set_through_the_inline_api() {
    let st = BarrierState::new(1);
    let heap = FakeHeap::new();
    let host = heap.cell(1);
    let cell = cell_value(heap.cell(3));
    let slot = Slot::new(Value::UNDEFINED);
    slot.set(host, cell, &st);
    slot.set(host, Value::int32(9), &st);
    assert_eq!(slot.get().as_int32(), Some(9));
    assert_eq!(st.slow_calls(), 2);
    st.set_threshold(BarrierState::OFF);
    slot.set(host, cell, &st);
    assert_eq!(st.slow_calls(), 2);
    assert!(slot.get().is_cell());
}

#[test]
fn cell_test_matches_value_is_cell() {
    let heap = FakeHeap::new();
    let cell = cell_value(heap.cell(1));
    for v in non_cells().into_iter().chain([cell]) {
        assert_eq!(is_cell_bits(v.to_bits()), v.is_cell());
    }
}
