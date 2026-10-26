#![allow(dead_code)] // m4-stub
//! Construct sites (owner M4-P1 OBJ-ALLOC; OI-05, V9-D8, V9-A8): the Call slot (0) of a
//! `Construct*` site keeps the target (weak) and a construct-site handler: an IC_HANDLER cell of
//! handler kind `CONSTRUCT_SITE` (`ic::hk`; +16 int32 count, +24 initial shape (weak), +32
//! int32 `prototype` field index), or an int32 count (IC_LAYOUT 2, `profile.rs` table). A hit
//! is a callee compare, one field load, a shape-proto compare, the allocation and the frame push
//! with new.target; no `Vec` on a hit.
//!
//! ```text
//! interp/ops.rs   Construct           -> fast::<false>   (M4-P0 hook)
//!                 ConstructWithSpread -> fast::<true>
//! interp/slow/call.rs construct branch, after `this` exists -> miss
//! ```
//!
//! M4-P0 stubs: [`fast`] returns `Flow::Cold`, so every construct takes `slow::call` as before;
//! [`miss`] records nothing.

use crate::call::CallSite;
use crate::interp::dsl::{Count, FSlot, R, RList};
use crate::interp::{Cx, Flow, St};

/// `Construct` (`SPREAD` false) or `ConstructWithSpread` (true): the constructor in `ctor`,
/// the arguments in `args` (`argc` registers), new.target in the accumulator, Call slot `s`.
#[inline(always)]
pub(crate) fn fast<const SPREAD: bool>(
    _st: &mut St<'_>,
    _ctor: R,
    _args: RList,
    _argc: Count,
    _s: FSlot,
) -> Flow {
    Flow::Cold
}

/// The cold construct path of `site` in frame `fp` (`slow::call`, construct branch) made the
/// receiver `this` (the hole for a derived constructor) for the bytecode constructor `callee`
/// with `new_target`. GC point allowed: every Value here is in a caller register or a Rust
/// local.
#[inline]
pub(crate) fn miss(
    _cx: &mut Cx,
    _fp: *mut u64,
    _site: &CallSite,
    _callee: u64,
    _new_target: u64,
    _this: u64,
) {
}
