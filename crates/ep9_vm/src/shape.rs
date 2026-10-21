//! Shapes (H8, H17): hidden classes for JS objects. Week-4 stub; P1 builds the shapes core
//! and P2 moves objects onto it.
//!
//! Hooks the rest of the VM already calls:
//!
//! | Hook | Called from | Job |
//! |---|---|---|
//! | [`heap_config`] | `Isolate::new`, before the heap exists | ask for the shape space (H08-Q1) |
//! | [`register_kinds`] | `object::register_kinds` | SHAPE, SLOTS, DESCRIPTORS, DESC_INDEX, TRANSITIONS, PROTO_INFO |
//! | [`init`] | `Runtime::new` | register the ShapeState root range |
//!
//! `Cx::proto_epoch` points at [`ShapeState::proto_epoch`], so a host call or a nested run
//! that changes a prototype is seen at once (never a stale copy).
#![allow(dead_code)] // wk4-stub

use core::cell::Cell;

use ep9_heap::{Heap, HeapConfig};

use crate::runtime::Runtime;

/// Per-isolate shape state (a `Runtime` field).
#[derive(Debug, Default)]
pub(crate) struct ShapeState {
    /// Bumped on every structural change of a prototype (OM-8); prototype IC handlers compare
    /// it.
    pub proto_epoch: Cell<u32>,
}

/// The heap settings the shape space needs. Identity until the object layout moves to
/// shapes.
pub(crate) fn heap_config(cfg: HeapConfig) -> HeapConfig {
    cfg
}

/// Registers the shape cell kinds.
pub(crate) fn register_kinds(_heap: &Heap) {}

/// Runs once in `Runtime::new`, after the kinds are registered and before the realm exists.
pub(crate) fn init(_rt: &Runtime) {}
