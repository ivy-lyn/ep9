//! Store barrier, version 2: the JSC Riptide form decided for H11 (03-gc §3).
//!
//! ```text
//! store(host, slot, v):                      same shape in T0, Rust, T1-T3, both AOT code kinds
//!     if is_cell(v) && host.gc_state <= threshold     // u8 values, unsigned compare
//!         barrier_slow(host, slot, v)                 // cold, out of line; it does the store
//!     else
//!         slot.store(v, Relaxed)
//! ```
//!
//! | gc_state (word0 byte 4) | Value | Meaning |
//! |---|---|---|
//! | [`gc_state::BLACK`] | 1 | Old (mark bit stuck) and not remembered. In S2: scanned |
//! | [`gc_state::WHITE`] | 2 | Young: allocated since the last GC. Every allocation writes WHITE |
//! | [`gc_state::GREY`] | 3 | Remembered (S1) or queued for a revisit (S2) |
//!
//! | Threshold | Value | Stage | Effect |
//! |---|---|---|---|
//! | [`threshold::OFF`] | 0 | S0 | No host fires |
//! | [`threshold::GEN`] | 1 | S1, S2 between cycles | BLACK hosts fire |
//! | [`threshold::MARKING`] | 0xFF | S2 while marking runs | Every host fires |
//!
//! - **Filter on the new value only.** A store that deletes an edge needs no barrier: the
//!   final fixpoint rescans the roots (retreating wavefront, not SATB).
//! - **The slow path does the store.** While marking runs, S2 will do `store (Release);
//!   fence (SeqCst); load gc_state` there. In S0 it only stores.
//! - **Emitter rule.** No GC point (call, safepoint poll, allocation slow path) between the
//!   check and the store; a loaded threshold or gc_state is never reused across a GC point.
//! - **Elision.** E1: `v` is known not to be a cell. E2: the host was allocated in this code
//!   region, no GC point ran since, and it has not escaped ([`Slot::init`]). E3: stores into
//!   roots (VM stack, handles, scratch roots, unit RW cells).
//!
//! The fast path is 4 instructions plus the cell test on aarch64 (`and`, `tst`, `b.ne`,
//! `ldrb`, `ldrb`, `cmp`, `b.ls`, then `str`). `disasm-barrier.sh` in this crate checks it on
//! the release build of [`probe::ep9_probe_barrier`].

use core::sync::atomic::{
    AtomicU8, AtomicU64,
    Ordering::{Relaxed, Release},
};

use ep9_value::encoding::NOT_CELL_MASK;
use ep9_value::{ShapeId, Value, Word0};

/// Barrier version for the unit header (H11, H35). It covers the fast-path shape (check, then
/// a slow path that stores), the new-value filter and `NOT_CELL_MASK`, the gc_state offset and
/// values, the threshold offset, the slow-path entries [`barrier_slow`] and
/// [`barrier_slow_shape`], the emitter rule, elisions E1-E3 and the TLAB offsets
/// ([`crate::Heap::TLAB_CURSOR_OFFSET`]). Slow-path semantics need no new version.
pub const BARRIER_VERSION: u32 = 2;

/// Cell states in word0 byte 4 ([`Word0::GC_STATE_OFFSET`]).
pub mod gc_state {
    /// Old and not remembered; in S2: scanned.
    pub const BLACK: u8 = 1;
    /// Allocated since the last GC.
    pub const WHITE: u8 = 2;
    /// Remembered (S1) or queued for a revisit (S2).
    pub const GREY: u8 = 3;
}

/// Values of the isolate's barrier threshold. A host fires when `gc_state <= threshold`.
pub mod threshold {
    /// S0: no host fires (every gc_state is at least 1).
    pub const OFF: u8 = 0;
    /// S1 and S2 between cycles: BLACK hosts fire.
    pub const GEN: u8 = 1;
    /// S2 while marking runs: every host fires.
    pub const MARKING: u8 = 0xFF;
}

/// The barrier field of the isolate. JIT and AOT code read `threshold` as one byte at a fixed
/// offset from the isolate register; [`crate::Heap`] keeps this struct at offset 0.
#[repr(C)]
#[derive(Debug)]
pub struct BarrierState {
    threshold: AtomicU8,
}

const _: () = assert!(core::mem::offset_of!(BarrierState, threshold) == 0);

impl BarrierState {
    /// Byte offset of the threshold inside this struct.
    pub const THRESHOLD_OFFSET: usize = 0;

    pub const fn new(threshold: u8) -> BarrierState {
        BarrierState {
            threshold: AtomicU8::new(threshold),
        }
    }

    #[inline(always)]
    pub fn threshold(&self) -> u8 {
        self.threshold.load(Relaxed)
    }

    /// Changes the threshold. Only the GC does this, at a safepoint.
    pub fn set_threshold(&self, threshold: u8) {
        self.threshold.store(threshold, Relaxed);
    }
}

/// A GC-visible heap field that holds a `Value`. Every access is a Relaxed atomic (H11).
///
/// A heap slot never holds the bits 0: allocation pre-fills every word of a new cell with
/// `undefined` (H06-empty).
#[repr(transparent)]
#[derive(Debug)]
pub struct Slot(AtomicU64);

const _: () = assert!(size_of::<Slot>() == 8 && align_of::<Slot>() == 8);

impl Slot {
    pub const fn new(v: Value) -> Slot {
        Slot(AtomicU64::new(v.to_bits()))
    }

    #[inline(always)]
    pub fn get(&self) -> Value {
        // SAFETY: a heap slot holds bits from `Value::to_bits` or the allocator's pre-fill
        // (`undefined`), so they are valid and not 0. A cell pattern points to a cell that the
        // GC keeps live while this slot is reachable (the host is live while `&self` is).
        unsafe { Value::from_bits_unchecked(self.0.load(Relaxed)) }
    }

    /// The raw bits, for the GC and for code that copies values without decoding them.
    #[inline(always)]
    pub fn bits(&self) -> u64 {
        self.0.load(Relaxed)
    }

    /// Stores `v` with the barrier. `host` is the header of the cell that holds this slot, and
    /// `state` is the heap's [`BarrierState`].
    #[inline(always)]
    pub fn set(&self, host: &Word0, v: Value, state: &BarrierState) {
        if is_cell_bits(v.to_bits()) && host.gc_state() <= state.threshold() {
            barrier_slow(host, self, v);
        } else {
            self.0.store(v.to_bits(), Relaxed);
        }
    }

    /// Stores `v` with no barrier. Allowed only under an elision rule: E1 (`v` is not a cell),
    /// E2 (the host was allocated with no GC point since and has not escaped), or E3 (the slot
    /// is a root).
    #[inline(always)]
    pub fn init(&self, v: Value) {
        self.0.store(v.to_bits(), Relaxed);
    }

    /// GC-internal store (weak clearing). Not a mutator store.
    #[inline(always)]
    pub(crate) fn gc_store(&self, v: Value) {
        self.0.store(v.to_bits(), Relaxed);
    }
}

/// The cell test on raw bits (JSC `isCell`).
#[inline(always)]
pub const fn is_cell_bits(bits: u64) -> bool {
    bits & NOT_CELL_MASK == 0
}

/// Installs a new ShapeId in word0 with the barrier (03-gc §3 "Shape install"). The value is
/// a shape cell, so the cell test is known to pass and is skipped.
#[inline(always)]
pub fn set_shape(host: &Word0, id: ShapeId, state: &BarrierState) {
    if host.gc_state() <= state.threshold() {
        barrier_slow_shape(host, id);
    } else {
        host.set_shape_id(id);
    }
}

/// Barrier slow path. JIT and AOT code reach it through the isolate table (Runtime
/// relocation), so it uses the C ABI. It never allocates on the GC heap and never reaches a
/// safepoint.
///
/// | Stage | Does |
/// |---|---|
/// | S0 (now) | The store (Release). Logs nothing |
/// | S1 | Store; LOS host: dirty the 512-byte card; BLACK host: set GREY, push to the remembered set |
/// | S2 | Store (Release); `fence(SeqCst)`; re-read gc_state; re-grey a BLACK marked host |
#[cold]
#[inline(never)]
pub extern "C" fn barrier_slow(host: &Word0, slot: &Slot, v: Value) {
    let _ = host;
    #[cfg(test)]
    slow_counter::bump();
    slot.0.store(v.to_bits(), Release);
}

/// Slow path of [`set_shape`]. S0: the store. S1 and S2 log the host as in [`barrier_slow`].
#[cold]
#[inline(never)]
pub extern "C" fn barrier_slow_shape(host: &Word0, id: ShapeId) {
    #[cfg(test)]
    slow_counter::bump();
    host.set_shape_id(id);
}

/// Per-thread count of slow-path calls, for the tests only.
#[cfg(test)]
pub(crate) mod slow_counter {
    use std::cell::Cell;

    std::thread_local! {
        static CALLS: Cell<u64> = const { Cell::new(0) };
    }

    pub(crate) fn bump() {
        CALLS.with(|c| c.set(c.get() + 1));
    }

    pub(crate) fn get() -> u64 {
        CALLS.with(Cell::get)
    }
}

/// Disassembly probes for SP-Value (09-milestones §4) and `disasm-barrier.sh`.
/// `#[unsafe(no_mangle)]` keeps each one as an exported symbol in release builds.
pub mod probe {
    use super::*;

    /// The decided barrier (`Slot::set`).
    #[unsafe(no_mangle)]
    pub extern "C" fn ep9_probe_barrier(host: &Word0, slot: &Slot, v: Value, st: &BarrierState) {
        slot.set(host, v, st);
    }

    /// The shape-install barrier (`set_shape`): no cell test.
    #[unsafe(no_mangle)]
    pub extern "C" fn ep9_probe_barrier_shape(host: &Word0, id: ShapeId, st: &BarrierState) {
        set_shape(host, id, st);
    }

    /// An elided store (`Slot::init`): one `str`.
    #[unsafe(no_mangle)]
    pub extern "C" fn ep9_probe_slot_init(slot: &Slot, v: Value) {
        slot.init(v);
    }
}
