#![allow(dead_code)] // m5-stub
//! Field representation and const bits (owner M5-P2 CONSTREP; the H17 "Representation and
//! const bits" row, C-02): each fast field has a representation, which only moves up, and a
//! const bit that the second store clears. Generalizing a field in place pops its Dynamic fuse
//! (descriptor kind 3, `fuse.rs`).
//!
//! ```text
//!  details bits 4-6 (shape::rep): NONE 0 -> INT32 1 -> DOUBLE 2 -> ANY 4
//!                                 NONE 0 -> HEAP 3 -> ANY 4
//!  details bit 7 (shape::DETAILS_CONST): the field held one value since it was added
//!  hook: props::field_set -> before_field_store
//! ```
//!
//! M5-P0 stub: every field reads as `rep::NONE` with no const bit (the details bits are 0)
//! and the hook does nothing.

pub(crate) use crate::shape::rep;

/// The representation the value `v` needs (`shape::rep` values). M5-P0 stub: `rep::ANY`.
#[inline(always)]
pub(crate) fn rep_of(_v: u64) -> i32 {
    rep::ANY
}

/// The hook of a store of `v` into fast field `f` of an object with shape `s`
/// (`props::field_set`), before the store. M5-P0 stub: no change. Not a GC point.
#[inline(always)]
pub(crate) fn before_field_store(_rt: &crate::runtime::Runtime, _s: u64, _f: usize, _v: u64) {}
