#![allow(dead_code)] // m5-stub
//! Slack tracking (owner M5-P2 SLACK; the H17 "Slack tracking" row): the first instances of a
//! root shape get a generous in-object capacity; after [`TRACKED_INSTANCES`] of them the root
//! shape's capacity becomes the largest field count seen, and the construct site keeps that
//! final shape. A live object never shrinks (H21 P5).
//!
//! ```text
//!  root shape flags bits 4-7 (shape::flag::SLACK_SHIFT, SLACK_MASK): instances left to track
//!  hooks: object::new_object -> initial_capacity   construct_ic::fast -> instance_shape
//! ```
//!
//! M5-P0 stub: both hooks return today's value (capacity 4 or 8; the cached shape), so nothing
//! changes. SLACK edits `shape.rs` only through these hooks.

/// The instances of a root shape that get the generous capacity (est., H17 slack row).
pub(crate) const TRACKED_INSTANCES: u8 = 7;

/// The in-object capacity of a new ordinary object with prototype `proto`, given today's
/// default `cap` (`object::new_object`). M5-P0 stub: `cap`. Not a GC point.
#[inline(always)]
pub(crate) fn initial_capacity(_rt: &crate::runtime::Runtime, _proto: u64, cap: usize) -> usize {
    cap
}

/// The shape of the receiver a construct-site hit allocates, given the shape the site cached
/// (`construct_ic::fast`). M5-P0 stub: `shape`. Not a GC point.
#[inline(always)]
pub(crate) fn instance_shape(_rt: &crate::runtime::Runtime, shape: u64) -> u64 {
    shape
}
