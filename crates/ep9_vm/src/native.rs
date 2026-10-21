#![allow(dead_code)] // m2-stub
//! The native entry for spike-made code (Spike B, D21; feature `spike-native` only, never in
//! the ep9 graph: dep-guards check 11). Owners: P1-NATIVE-ENTRY, then P2-SPIKE-B.
//!
//! ```text
//! T0 call (slow/call.rs) --Baseline / AotBaseline slot--> push_frame, publish fp
//!     --> extern "C" (aarch64) / extern "sysv64" (x86-64) slot entry (cx, fp)
//! native code reads two Cx fields at fixed offsets:
//!     Cx::native_limit    the prologue's native stack compare (= Runtime::native_limit)
//!     Cx::runtime_table   -> RUNTIME_TABLE: the runtime calls native code may make
//! ```
//!
//! P0 stub: the table has no entries and nothing calls native code. P1-NATIVE-ENTRY fills
//! the table with functions that never unwind through native frames (poll, call_any, the add,
//! inc and compare slow paths, stack_overflow and throw).

/// The runtime functions native code calls, at fixed offsets (`#[repr(C)]`), reached through
/// `Cx::runtime_table`. Stub: no entries yet.
#[repr(C)]
#[derive(Debug)]
pub(crate) struct RuntimeTable {}

/// The one runtime table of the process; every isolate's `Cx::runtime_table` points at it.
pub(crate) static RUNTIME_TABLE: RuntimeTable = RuntimeTable {};
