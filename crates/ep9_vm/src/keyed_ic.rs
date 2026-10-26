#![allow(dead_code)] // m4-stub
//! Keyed access (owner M4-P3 KEYED; OI-04): the fast bodies of the keyed opcodes. They read
//! the bits that `profile.rs` writes into the LoadKeyed, StoreKeyed, DefineKeyedOwn and
//! HasKeyed slots (2, 4, 6, 10; IC_LAYOUT 2): element form seen, key types (int32, string,
//! symbol), out of bounds, hole read.
//!
//! ```text
//! interp/ops.rs (M4-P0 hooks; each after profile::record_keyed)
//!   GetKeyedProperty          array-only inline path, then -> fast_get
//!   SetKeyedProperty          -> fast_set
//!   DefineKeyedOwnProperty    -> fast_define_own
//!   StaInArrayLiteral         -> fast_sta_in_array_literal
//!   TestIn                    -> fast_has
//! ```
//!
//! M4-P0 stubs: each returns `Flow::Cold` and changes nothing, so T0 does not change. A fast
//! body sets `st.acc` (a load or `in`) and returns `Flow::Next` on a hit; it never allocates
//! and never calls (H16:103).

use crate::interp::dsl::FSlot;
use crate::interp::{Flow, St};

/// `o[k]` (LoadKeyed slot `s`), after the array-only path of `GetKeyedProperty` missed.
#[inline(always)]
pub(crate) fn fast_get(_st: &mut St<'_>, _o: u64, _k: u64, _s: FSlot) -> Flow {
    Flow::Cold
}

/// `o[k] = v` (StoreKeyed slot `s`).
#[inline(always)]
pub(crate) fn fast_set(_st: &mut St<'_>, _o: u64, _k: u64, _v: u64, _s: FSlot) -> Flow {
    Flow::Cold
}

/// CreateDataProperty `o[k] = v` with the opcode's `flags` (DefineKeyedOwn slot `s`).
#[inline(always)]
pub(crate) fn fast_define_own(
    _st: &mut St<'_>,
    _o: u64,
    _k: u64,
    _v: u64,
    _flags: u8,
    _s: FSlot,
) -> Flow {
    Flow::Cold
}

/// An array literal element `a[i] = v` (StoreInArrayLiteral slot `s`).
#[inline(always)]
pub(crate) fn fast_sta_in_array_literal(
    _st: &mut St<'_>,
    _a: u64,
    _i: u64,
    _v: u64,
    _s: FSlot,
) -> Flow {
    Flow::Cold
}

/// `k in o` (HasKeyed slot `s`): the result goes to the accumulator.
#[inline(always)]
pub(crate) fn fast_has(_st: &mut St<'_>, _k: u64, _o: u64, _s: FSlot) -> Flow {
    Flow::Cold
}
