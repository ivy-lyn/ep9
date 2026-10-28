#![allow(dead_code)] // m5-stub
//! The deopt core (owner M5-P1 SPIKEC-5, then SPIKEC-6; H29, V10-D29, V10-D32, SC WI-SC2 and
//! WI-SC3): the H29 metadata codec, the frame rebuild and the exit counters. It has no `cfg`:
//! every build has it and the jitless build tests it (C-10, doc 13:371). Only the
//! RuntimeTable glue (`native.rs` `rt_deopt`, entry 10) and the install with metadata stay
//! behind `spike-native`.
//!
//! ```text
//!  guard fails in native code -> exit stub -> shared exit -> RuntimeTable::deopt (entry 10)
//!    rt_deopt (native.rs, spike-native) -> SideTable::lookup(code start) -> DeoptInfo
//!    -> decode the record at the pc offset (H29 form) -> rebuild the VM frame, count the exit
//!    -> T0 resumes in the same VM frame
//! ```
//!
//! M5-P0 stub: the side table is empty and the entry throws "deopt not built" (the Spike C
//! trap of month 4). DEOPT_META_VERSION stays 0 in month 5 (V10-D29).

/// The per-isolate deopt side table (V10-D32; `Runtime::deopt`): code start -> the code's
/// deopt metadata bytes and its exit counters, filled at install. Rust memory that holds no
/// Value across a GC point (plan rule 8). Not in `CodeRegistry`: a `dispatch.rs` change would
/// need an SP-Dispatch row (rule 22).
pub(crate) struct SideTable {
    _private: (),
}

impl SideTable {
    /// The empty table of a new isolate.
    pub(crate) fn new() -> SideTable {
        SideTable { _private: () }
    }

    /// True if no code has metadata here (M5-P0: always).
    pub(crate) fn is_empty(&self) -> bool {
        true
    }
}
