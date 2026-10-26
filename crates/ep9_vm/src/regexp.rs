#![allow(dead_code)] // m4-stub
//! RegExp objects in the VM (owners M4-P2 RX0A, then M4-P3 REGEXP1; V9-D15, V9-A7): the
//! RegExp object (a SLOT_OBJECT of `ObjectClass::RegExp`), `CreateRegExpLiteral` and the
//! per-isolate program table.
//!
//! A compiled program (`ep9_regexp`) is Rust memory with no Value in it. It lives in
//! `Runtime::regexp_programs` ([`ProgramTable`], made in `Runtime::new`); a RegExp object holds
//! its int32 index (not a native pointer). A weak source that the table registers in
//! [`ProgramTable::new`] frees the entry when its RegExp cell dies (no Rust `Drop` runs from
//! the heap). `runtime.rs` has no owner after M4-P0, so the owners change only this file.
//!
//! M4-P0 stubs: the table is empty and registers nothing; [`create_literal`] throws the v0
//! "not implemented" Error that the `CreateRegExpLiteral` cold body threw before.

use ep9_heap::Heap;

use crate::interp::{Cx, slow};

/// The compiled programs of the isolate, indexed by the int32 a RegExp object holds.
pub(crate) struct ProgramTable {
    _private: (),
}

impl ProgramTable {
    /// The empty table of a new isolate. `heap` is the isolate's heap, before its first
    /// allocation: the owner registers the table's weak source here (`Heap::add_weak_source`).
    pub(crate) fn new(_heap: &Heap) -> ProgramTable {
        ProgramTable { _private: () }
    }
}

/// `CreateRegExpLiteral` (the `CreateRegExpLiteral` cold body; pattern constant `pattern`,
/// Literal slot `slot`, `flags` the RegExp flag bits) at `pc` in frame `fp`: the new RegExp
/// object, or 0 with the exception pending.
#[cold]
#[inline(never)]
pub(crate) fn create_literal(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    _pattern: u32,
    _slot: u32,
    _flags: u16,
) -> u64 {
    slow::not_impl(cx, fp, pc, "RegExp literals")
}
