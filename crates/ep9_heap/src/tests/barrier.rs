//! Barrier v2 (H11 decided): `is_cell(v) && host.gc_state <= threshold` -> slow path, which
//! does the store.

use ep9_value::{ShapeId, Value, Word0};

use super::*;
use crate::barrier::probe::{ep9_probe_barrier, ep9_probe_barrier_shape, ep9_probe_slot_init};
use crate::barrier::{BarrierState, is_cell_bits, slow_counter};
use crate::{BARRIER_VERSION, gc_state, threshold};

fn non_cells() -> [Value; 8] {
    [
        Value::int32(0),
        Value::int32(-1),
        Value::double(0.5),
        Value::double(f64::NAN),
        Value::UNDEFINED,
        Value::NULL,
        Value::TRUE,
        Value::EMPTY,
    ]
}

/// Runs one store through the decided probe; returns whether the slow path ran. Checks that
/// the store landed either way.
fn slow_taken(host: &Word0, old: Value, v: Value, st: &BarrierState) -> bool {
    let slot = Slot::new(old);
    let before = slow_counter::get();
    ep9_probe_barrier(host, &slot, v, st);
    assert_eq!(slot.get().to_bits(), v.to_bits(), "the store must land");
    match slow_counter::get() - before {
        0 => false,
        1 => true,
        n => panic!("{n} slow calls for one store"),
    }
}

#[test]
fn decided_constants() {
    assert_eq!(BARRIER_VERSION, 2);
    assert_eq!(
        (gc_state::BLACK, gc_state::WHITE, gc_state::GREY),
        (1, 2, 3)
    );
    assert_eq!(
        (threshold::OFF, threshold::GEN, threshold::MARKING),
        (0, 1, 0xFF)
    );
    assert_eq!(Word0::GC_STATE_OFFSET, 4);
    assert_eq!(BarrierState::THRESHOLD_OFFSET, 0);
    assert_eq!(Heap::BARRIER_THRESHOLD_OFFSET, 0);
    assert_eq!((Heap::TLAB_CURSOR_OFFSET, Heap::TLAB_LIMIT_OFFSET), (8, 16));
}

#[test]
fn heap_field_offsets_hold_at_run_time() {
    let heap = test_heap(|_| {});
    let base = core::ptr::from_ref(&heap).addr();
    assert_eq!(
        core::ptr::from_ref(heap.barrier()).addr(),
        base + Heap::BARRIER_THRESHOLD_OFFSET
    );
    heap.barrier().set_threshold(0x5A);
    // SAFETY: offset 0 of the heap is the threshold byte (asserted above), read as an atomic.
    let raw = unsafe {
        (*core::ptr::from_ref(&heap).cast::<core::sync::atomic::AtomicU8>())
            .load(core::sync::atomic::Ordering::Relaxed)
    };
    assert_eq!(raw, 0x5A);
    heap.barrier().set_threshold(threshold::OFF);
}

/// The full decision table: every gc_state byte x the stage thresholds x cell/non-cell values.
#[test]
fn filter_is_new_value_and_header_compare() {
    let heap = test_heap(|_| {});
    let target = new_node(&heap, 1, 1).to_value();
    for th in [
        threshold::OFF,
        threshold::GEN,
        2,
        0x7F,
        0xFE,
        threshold::MARKING,
    ] {
        let st = BarrierState::new(th);
        for state in 0..=u8::MAX {
            let host = Word0::new(ShapeId::new(8), state, 0, 0);
            // New value is a cell: slow iff state <= threshold (unsigned).
            assert_eq!(
                slow_taken(&host, Value::UNDEFINED, target, &st),
                state <= th,
                "threshold {th} state {state}"
            );
            for nc in non_cells() {
                // New value not a cell: never slow, even over an old cell (no SATB).
                assert!(!slow_taken(&host, target, nc, &st), "{nc:?} over a cell");
                assert!(!slow_taken(&host, Value::int32(3), nc, &st));
            }
        }
    }
}

#[test]
fn stage_thresholds_pick_the_documented_hosts() {
    let heap = test_heap(|_| {});
    let v = new_node(&heap, 1, 1).to_value();
    let black = Word0::new(ShapeId::new(8), gc_state::BLACK, 0, 0);
    let white = Word0::new(ShapeId::new(8), gc_state::WHITE, 0, 0);
    let grey = Word0::new(ShapeId::new(8), gc_state::GREY, 0, 0);
    let off = BarrierState::new(threshold::OFF);
    let gen_ = BarrierState::new(threshold::GEN);
    let marking = BarrierState::new(threshold::MARKING);
    for h in [&black, &white, &grey] {
        assert!(!slow_taken(h, Value::NULL, v, &off), "S0: no host fires");
        assert!(
            slow_taken(h, Value::NULL, v, &marking),
            "marking: every host"
        );
    }
    assert!(slow_taken(&black, Value::NULL, v, &gen_), "S1: BLACK fires");
    assert!(
        !slow_taken(&white, Value::NULL, v, &gen_),
        "S1: WHITE does not"
    );
    assert!(
        !slow_taken(&grey, Value::NULL, v, &gen_),
        "S1: GREY is remembered"
    );
}

#[test]
fn slot_set_on_real_cells() {
    let heap = test_heap(|_| {});
    let host = new_node(&heap, 1, 2);
    let target = new_node(&heap, 2, 0);
    // SAFETY: both cells are live (locals on this stack).
    let hdr = unsafe { host.header() };
    assert_eq!(hdr.gc_state(), gc_state::WHITE, "allocation writes WHITE");
    let before = slow_counter::get();
    store(&heap, host, 0, target.to_value());
    assert_eq!(slow_counter::get(), before, "threshold OFF in S0");
    heap.barrier().set_threshold(threshold::MARKING);
    store(&heap, host, 1, target.to_value());
    store(&heap, host, 0, Value::int32(4));
    assert_eq!(slow_counter::get(), before + 1, "only the cell store fired");
    heap.barrier().set_threshold(threshold::OFF);
    assert_eq!(slot(host, 0).get().as_int32(), Some(4));
    assert_eq!(slot(host, 1).get().to_bits(), target.to_value().to_bits());
}

#[test]
fn shape_install_barrier() {
    let st_off = BarrierState::new(threshold::OFF);
    let st_gen = BarrierState::new(threshold::GEN);
    for state in [gc_state::BLACK, gc_state::WHITE, gc_state::GREY] {
        let host = Word0::new(ShapeId::new(8), state, 0, 0);
        let before = slow_counter::get();
        ep9_probe_barrier_shape(&host, ShapeId::new(16), &st_off);
        assert_eq!(host.shape_id(), ShapeId::new(16));
        ep9_probe_barrier_shape(&host, ShapeId::new(24), &st_gen);
        assert_eq!(host.shape_id(), ShapeId::new(24));
        let fired = slow_counter::get() - before;
        assert_eq!(fired, u64::from(state == gc_state::BLACK), "state {state}");
    }
}

#[test]
fn init_never_fires() {
    let heap = test_heap(|_| {});
    let v = new_node(&heap, 1, 0).to_value();
    let s = Slot::new(Value::UNDEFINED);
    let before = slow_counter::get();
    ep9_probe_slot_init(&s, v);
    assert_eq!(s.get().to_bits(), v.to_bits());
    assert_eq!(slow_counter::get(), before);
}

#[test]
fn cell_test_matches_value_is_cell() {
    let heap = test_heap(|_| {});
    let cell = new_node(&heap, 1, 0).to_value();
    for v in non_cells().into_iter().chain([cell]) {
        assert_eq!(is_cell_bits(v.to_bits()), v.is_cell());
    }
    assert!(
        is_cell_bits(0),
        "0 passes the cell test: why heap slots never hold 0"
    );
}
