#![allow(dead_code)] // m4-stub
//! WeakRef and FinalizationRegistry (owner M4-P4 WEAKREF; GFF-WEAKREF, V9-D23): SLOT_OBJECT
//! classes (`ObjectClass::WeakRef` 11, `ObjectClass::FinalizationRegistry` 12) with one
//! internal WEAK_CELL kind (80) for the weak target, the `[[KeptAlive]]` list (§9.10,
//! AddToKeptObjects, ClearKeptObjects) and the cleanup jobs.
//!
//! The isolate's kept list is `Runtime::kept_objects` ([`KeptObjects`], made in
//! `Runtime::new`): one root slot that holds a heap list cell (plan rule 8: no Rust `Vec` of
//! Values), undefined until the first WeakRef. `runtime.rs` has no owner after M4-P0, so the
//! owner changes only this file.

use core::cell::Cell;
use std::rc::Rc;

use ep9_heap::Heap;

use crate::frame::UNDEFINED;

/// The `[[KeptAlive]]` list of the isolate: a strong root slot (a root source of the heap) that
/// holds a heap list cell, or undefined.
pub(crate) struct KeptObjects {
    slot: Rc<Cell<u64>>,
}

impl KeptObjects {
    /// The empty list of a new isolate; registers the root slot with `heap`.
    pub(crate) fn new(heap: &Heap) -> KeptObjects {
        let slot = Rc::new(Cell::new(UNDEFINED));
        let s = Rc::clone(&slot);
        heap.add_root_source(move |t| t.visit_bits(s.get()));
        KeptObjects { slot }
    }

    /// The list cell, or undefined.
    pub(crate) fn get(&self) -> u64 {
        self.slot.get()
    }

    /// Replaces the list cell (a root slot: no barrier, E3).
    pub(crate) fn set(&self, list: u64) {
        self.slot.set(list);
    }
}
