//! Pre-store barrier fast path (H11, 03-gc §3). Prototype for SP-Value; the slow path only
//! counts calls until S1/S2 give it a remembered set and an SATB queue.
//!
//! ```text
//! Slot::set(host, slot, new, state):
//!     if host.word0.gc_state <= state.threshold        // header byte vs isolate byte
//!         if is_cell(old) || is_cell(new)              // old = slot value before the store
//!             barrier_slow(host, state)                // cold, out of line
//!     slot = new                                       // Relaxed
//! ```
//!
//! Which value the cell test reads:
//!
//! | Filter       | Generations (S1: old host gets a new young ref) | SATB (S2: overwritten ref must stay marked) |
//! |--------------|-----------------------------------------------|---------------------------------------------|
//! | new (H11 text) | sound                                        | unsound: `host.f = 5` drops the old cell     |
//! | old (task)   | unsound: `host.f = young` over an int is missed | sound                                      |
//! | old or new   | sound                                          | sound                                       |
//!
//! `Slot::set` uses "old or new". The header check runs first, so the common fast path never
//! loads the old value. The probes in [`probe`] build all three forms for disassembly.

use core::sync::atomic::{AtomicU8, AtomicU64, Ordering::Relaxed};

use ep9_value::encoding::NOT_CELL_MASK;
use ep9_value::{Value, Word0};

/// Barrier version for the unit header (H11, H35). It covers the fast-path shape and the
/// threshold offset. Bump it when either changes; slow-path changes need no bump.
pub const BARRIER_VERSION: u32 = 1;

/// Barrier fields of the isolate. JIT and AOT code read `threshold` as one byte at a fixed
/// offset from the isolate register.
#[repr(C)]
#[derive(Debug)]
pub struct BarrierState {
    threshold: AtomicU8,
    slow_calls: AtomicU64,
}

const _: () = assert!(core::mem::offset_of!(BarrierState, threshold) == 0);

impl BarrierState {
    /// Byte offset of the threshold inside this struct.
    pub const THRESHOLD_OFFSET: usize = 0;
    /// Threshold that turns the barrier off (S0). Cell states start at 1, so none is <= 0.
    pub const OFF: u8 = 0;

    pub const fn new(threshold: u8) -> BarrierState {
        BarrierState {
            threshold: AtomicU8::new(threshold),
            slow_calls: AtomicU64::new(0),
        }
    }

    #[inline(always)]
    pub fn threshold(&self) -> u8 {
        self.threshold.load(Relaxed)
    }

    pub fn set_threshold(&self, threshold: u8) {
        self.threshold.store(threshold, Relaxed);
    }

    /// Number of slow-path calls so far (prototype counter).
    pub fn slow_calls(&self) -> u64 {
        self.slow_calls.load(Relaxed)
    }
}

/// A GC-visible heap field that holds a `Value`. Every access is a Relaxed atomic (H11).
#[repr(transparent)]
#[derive(Debug)]
pub struct Slot(AtomicU64);

impl Slot {
    pub const fn new(v: Value) -> Slot {
        Slot(AtomicU64::new(v.to_bits()))
    }

    #[inline(always)]
    pub fn get(&self) -> Value {
        // SAFETY: a Slot only ever holds bits from `Value::to_bits` (`new`, `set`), so they are
        // non-zero and valid; that makes building the Value sound. A cell pattern came from a
        // cell that was live when its Value was made (`from_cell_ptr` contract). Keeping that
        // cell live while a slot holds it is the caller's contract in S0 (no GC yet) and the
        // GC's once it traces heap slots.
        unsafe { Value::from_bits_unchecked(self.0.load(Relaxed)) }
    }

    /// Stores `new` with the pre-store barrier. `host` is the header of the cell that holds
    /// this slot.
    #[inline(always)]
    pub fn set(&self, host: &Word0, new: Value, state: &BarrierState) {
        if host.gc_state() <= state.threshold() {
            let old = self.0.load(Relaxed);
            if is_cell_bits(old) || is_cell_bits(new.to_bits()) {
                barrier_slow(host, state);
            }
        }
        self.0.store(new.to_bits(), Relaxed);
    }
}

/// The cell test on raw bits (JSC `isCell`).
#[inline(always)]
pub const fn is_cell_bits(bits: u64) -> bool {
    bits & NOT_CELL_MASK == 0
}

/// Slow path. JIT and AOT code reach it through the isolate table (Runtime relocation), so it
/// uses the C ABI. S0: no remembered set or SATB queue yet; it only counts.
#[cold]
#[inline(never)]
pub extern "C" fn barrier_slow(host: &Word0, state: &BarrierState) {
    let _ = host;
    state.slow_calls.fetch_add(1, Relaxed);
}

/// Disassembly probes for SP-Value (09-milestones §4) and `cargo xtask disasm-check`.
/// `#[unsafe(no_mangle)]` keeps each one as an exported symbol in release builds.
pub mod probe {
    use super::*;

    /// The barrier that `Slot::set` uses: header byte first, then "old or new is a cell".
    #[unsafe(no_mangle)]
    pub extern "C" fn ep9_probe_barrier(host: &Word0, slot: &Slot, new: Value, st: &BarrierState) {
        slot.set(host, new, st);
    }

    /// H11 as 03-gc §3 writes it: `is_cell(new) && host.gc_state <= threshold`.
    #[unsafe(no_mangle)]
    pub extern "C" fn ep9_probe_barrier_new(
        host: &Word0,
        slot: &Slot,
        new: Value,
        st: &BarrierState,
    ) {
        if is_cell_bits(new.to_bits()) && host.gc_state() <= st.threshold() {
            barrier_slow(host, st);
        }
        slot.0.store(new.to_bits(), Relaxed);
    }

    /// SATB form: `is_cell(old) && host.gc_state <= threshold`.
    #[unsafe(no_mangle)]
    pub extern "C" fn ep9_probe_barrier_old(
        host: &Word0,
        slot: &Slot,
        new: Value,
        st: &BarrierState,
    ) {
        if host.gc_state() <= st.threshold() && is_cell_bits(slot.0.load(Relaxed)) {
            barrier_slow(host, st);
        }
        slot.0.store(new.to_bits(), Relaxed);
    }
}
