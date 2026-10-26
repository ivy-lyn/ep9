#![allow(dead_code)] // m4-stub
//! Literal sites (owner M4-P1 OBJ-ALLOC; OI-03, the V9-D8 pattern, V9-A8): the Literal and
//! CloneObject feedback slots (15, 16; IC_LAYOUT 2, `profile.rs` table) keep the result shape
//! (weak) and an int32 count; no cell. A hit allocates with that shape and copies the
//! constants; computed or nested values stay generic.
//!
//! ```text
//! interp/ops.rs fast bodies (M4-P0 hooks)          cold path
//!   CreateObjectLiteral       -> fast_object       slow::literal  -> miss        (slow/literal.rs)
//!   CreateArrayLiteral        -> fast_array        slow::literal  -> miss
//!   CreateEmptyArrayLiteral   -> fast_empty_array  slow::literal  -> miss
//!   CreateEmptyObjectLiteral  -> fast_empty_object slow::literal  -> miss
//!   CloneObject               -> fast_clone        slow::clone_object -> miss_clone
//! ```
//!
//! M4-P0 stubs: every fast body returns `Flow::Cold` (T0 does not change) and every miss
//! records nothing. A fast body is `#[inline(always)]`, makes no runtime call and allocates
//! only on its hit path; `interp/ops.rs` has no owner after M4-P0 (V9-D5), so these signatures
//! are what the owner fills.

use crate::interp::dsl::{FSlot, K};
use crate::interp::slow::Lit;
use crate::interp::{Cx, Flow, St};

/// `CreateObjectLiteral` (boilerplate constant `boilerplate`, Literal slot `s`).
#[inline(always)]
pub(crate) fn fast_object(_st: &mut St<'_>, _boilerplate: K, _s: FSlot, _flags: u8) -> Flow {
    Flow::Cold
}

/// `CreateArrayLiteral` (boilerplate constant `boilerplate`, Literal slot `s`).
#[inline(always)]
pub(crate) fn fast_array(_st: &mut St<'_>, _boilerplate: K, _s: FSlot, _flags: u8) -> Flow {
    Flow::Cold
}

/// `CreateEmptyArrayLiteral` (Literal slot `s`).
#[inline(always)]
pub(crate) fn fast_empty_array(_st: &mut St<'_>, _s: FSlot) -> Flow {
    Flow::Cold
}

/// `CreateEmptyObjectLiteral` (no feedback slot).
#[inline(always)]
pub(crate) fn fast_empty_object(_st: &mut St<'_>) -> Flow {
    Flow::Cold
}

/// `CloneObject` of `source` (CloneObject slot `s`).
#[inline(always)]
pub(crate) fn fast_clone(_st: &mut St<'_>, _source: u64, _flags: u8, _s: FSlot) -> Flow {
    Flow::Cold
}

/// The cold path of a literal of `kind` (constant `k`) at `pc` in frame `fp` made `result`
/// (`slow::literal`, after the object exists; the slot is an operand of the instruction at
/// `pc`). GC point allowed: `result` is in a Rust local. Not called for a throw.
#[inline]
pub(crate) fn miss(_cx: &mut Cx, _fp: *mut u64, _pc: *const u8, _kind: Lit, _k: u32, _result: u64) {
}

/// The cold path of `CloneObject` at `pc` made `result` from `source` (`slow::clone_object`).
#[inline]
pub(crate) fn miss_clone(_cx: &mut Cx, _fp: *mut u64, _pc: *const u8, _source: u64, _result: u64) {}
