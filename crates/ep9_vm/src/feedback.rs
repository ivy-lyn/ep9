//! Feedback (H16): the lazy FeedbackVector of a function and its unit's slots. Week-4 stub;
//! P2 builds it.
//!
//! ```text
//! FunctionInfo.feedback ──> Slot in UnitFeedback (a strong root range, one per function)
//!                             └─ undefined, or a FEEDBACK cell: [key (weak) | value] per slot
//! ```
#![allow(dead_code)] // wk4-stub

use ep9_heap::Heap;

use crate::runtime::Runtime;
use crate::unit::LoadedUnit;

/// One function's feedback slot: a Value (undefined or its FEEDBACK cell) in the unit's root
/// range.
pub(crate) type Slot = ep9_heap::Slot;

/// The feedback slots of one unit (`LoadedUnit::feedback`), one per function. Empty until
/// the vectors exist.
#[derive(Default)]
pub(crate) struct UnitFeedback {
    slots: Box<[Slot]>,
}

impl UnitFeedback {
    /// The feedback of a unit with `functions` functions, made in `LoadedUnit::load` before
    /// the unit's first allocation.
    pub(crate) fn new(_rt: &Runtime, _functions: usize) -> UnitFeedback {
        UnitFeedback::default()
    }
}

/// The feedback slot of function `function_id` of `unit`, if it has one.
pub(crate) fn unit_slot(unit: &LoadedUnit, function_id: u32) -> Option<&Slot> {
    unit.feedback.slots.get(function_id as usize)
}

/// Registers the FEEDBACK cell kind.
pub(crate) fn register_kinds(_heap: &Heap) {}
