#![allow(dead_code)] // m4-stub
//! The instanceof cache (owner M4-P3 IC2; OI-07, H16-Q3, V9-A8): the InstanceOf slot (14;
//! IC_LAYOUT 2, `profile.rs` table) keeps the rhs shape (weak) and an instanceof handler (an
//! IC_HANDLER cell of handler kind `INSTANCEOF`, `ic::hk`: +16 validity cell, +24 undefined,
//! +32 int32 `prototype` field index) or an int32 state. The cache holds shapes and functions
//! weakly (H16 K7).
//!
//! `interp/ops.rs` `TestInstanceOf` calls [`fast`] (M4-P0 hook). M4-P0 stub: `Flow::Cold`, so
//! every `instanceof` takes the generic path (`property::instance_of`) as before.

use crate::interp::dsl::FSlot;
use crate::interp::{Flow, St};

/// `o instanceof c` (InstanceOf slot `s`): on a hit the boolean goes to the accumulator.
#[inline(always)]
pub(crate) fn fast(_st: &mut St<'_>, _o: u64, _c: u64, _s: FSlot) -> Flow {
    Flow::Cold
}
