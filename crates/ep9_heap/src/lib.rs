//! The GC heap of one isolate (03-gc): cage pages, allocation, the S0 collector, the store
//! barrier, conservative stack scanning, precise roots, the verifier and the root-range hook
//! for unit RW data.
//!
//! ```text
//!                       ep9_vm (object model, isolate)
//!   register_kind(id, {size, trace}) | alloc(kind, size) | Slot::set(host, v, heap.barrier())
//!   register_kind_in(id, kind, Space::Shape) | alloc_shape | shape_id_of | shape_cell
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
//!   |   verify:  heap verifier                                                             |
//!   +---------------------------------------------------------------------------------------+
//! ```
//!
//! | Stage | State |
//! |---|---|
//! | S0 | Built: stop-the-world, non-moving mark-sweep in mark-region style; lazy per-page sweep; bump allocation into free lines; LOS above 8 KiB; GC-stress; verifier; zapping |
//! | S1-S3 | Not built. The barrier fast path, gc_state values and TLAB offsets are already the S1/S2 ones |

pub mod barrier;
mod cell;
#[cfg(not(miri))]
mod css;
mod heap;
pub mod layout;
mod memory;
mod roots;
mod trace;
mod verify;

pub use barrier::{
    BARRIER_VERSION, BarrierState, Slot, barrier_slow, barrier_slow_shape, gc_state, set_shape,
    threshold,
};
pub use cell::{CellKind, CellPtr, CellSize, KindError, KindId, SizeFn, Space, TraceFn};
pub use ep9_platform::CageConfig;
pub use heap::{AllocError, GcStats, Heap, HeapConfig};
pub use memory::HeapError;
pub use roots::{GlobalHandle, RootId, RootStrength, ScratchRoots, WeakHandle};
pub use trace::{Tracer, WeakVisitor};
pub use verify::{VerifyError, VerifyReport};

#[cfg(test)]
mod tests;
