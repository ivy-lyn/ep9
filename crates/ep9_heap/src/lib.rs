//! The GC heap of one isolate (03-gc): cage pages, allocation, the S0 collector, the store
//! barrier, conservative stack scanning, precise roots, the verifier and the root-range hook
//! for unit RW data.
//!
//! ```text
//!                       ep9_vm (object model, isolate)
//!   register_kind(id, {size, trace}) | alloc(kind, size) | Slot::set(host, v, heap.barrier())
//!   register_kind_in(id, kind, Space::Shape) | alloc_shape | shape_id_of | shape_cell
//!   register_typed::<T: HeapLayout>()  (a Rust cell type; does not build if T needs Drop)
//!   new_global / scratch_roots / add_root_source / add_root_range / add_weak_source | collect
//!                                     |
//!   +---------------------------------v----------------------------------------------------+
//!   | Heap (repr(C): barrier threshold @0, TLAB cursor @8, TLAB limit @16, cage base @24) |
//!   |   layout:  256 KiB pages, in-page header + mark / VO bitmaps + line marks (H9)       |
//!   |   memory:  chunks of the ep9_platform cage (H7); LOS = own chunk run;                |
//!   |            optional shape region = the first chunks (ShapeId = offset, H08-Q1)       |
//!   |   trace:   Tracer (mark / verify), interior lookup, WeakVisitor                      |
//!   |   css:     spill trampoline + scan [sp, stack high), end rule (H10)                  |
//!   |   roots:   handles, ScratchRoots, root sources, root ranges, weak sources            |
//!   |   barrier: version 2, Riptide fast path, slow paths (H11)                            |
//!   |   remembered: S1 logs: per-page remembered lists, LOS cards (V9-D21)               |
//!   |   verify:  heap verifier (after a GC: checked mark + checked sweep); E2 check        |
//!   +---------------------------------------------------------------------------------------+
//! ```
//!
//! | Stage | State |
//! |---|---|
//! | S0 | Built: stop-the-world, non-moving mark-sweep in mark-region style; lazy per-page sweep; bump allocation into free lines; LOS above 8 KiB; GC-stress; verifier; zapping |
//! | S1 | Step 1 built behind [`HeapConfig::generational`] (default off; M4-P2): the barrier slow path logs BLACK hosts in per-page remembered lists and dirties 512-byte cards of LOS hosts; sticky marks (the sweep keeps the marks of the cells it keeps). Every collection is still a full GC; the minor GC is step 2 (M4-P3) |
//! | S2-S3 | Not built. The barrier fast path, gc_state values and TLAB offsets are already the S1/S2 ones |
//!
//! No Rust code runs when a cell dies (H12): no `Drop`, no finalizer. [`HeapLayout`] and
//! [`Heap::register_typed`] enforce this for cell kinds laid out as Rust types.

pub mod barrier;
mod cell;
#[cfg(not(miri))]
mod css;
mod heap;
pub mod layout;
mod memory;
mod remembered;
mod roots;
mod trace;
mod verify;

pub use barrier::{
    BARRIER_VERSION, BarrierState, Slot, barrier_slow, barrier_slow_shape, gc_state, set_shape,
    threshold,
};
pub use cell::{
    CellKind, CellPtr, CellSize, HeapLayout, KindError, KindId, SizeFn, Space, TraceFn,
};
pub use ep9_platform::CageConfig;
pub use heap::{AllocError, GcStats, Heap, HeapConfig, VerifyMode};
pub use memory::HeapError;
pub use roots::{GlobalHandle, RootId, RootStrength, ScratchRoots, WeakHandle};
pub use trace::{Tracer, WeakVisitor};
pub use verify::{
    E2_CHECK_UNDER_GC_STRESS, E2Check, VerifyError, VerifyReport, e2_check_init, e2_check_mode,
    e2_check_on, e2_reports, set_e2_check,
};

#[cfg(test)]
mod tests;
