//! Conservative scan of the native stack (H10, 03-gc §2).
//!
//! ```text
//! Heap::collect --> ep9_platform::with_spilled_registers (callee-saved regs pushed)
//!                     --> scan [sp, stack high) word by word
//!                           word in the used heap range?  (one subtract, one compare)
//!                             find_cell(w)      interior pointers via the VO bitmap / chunk table
//!                             find_cell(w - 1)  end rule: a one-past-end pointer keeps its cell
//!                           --> mark (pinned for this GC: S0-S2 never move cells)
//! ```
//!
//! - NaN-boxed values need no untagging: a cell `Value` is the raw address, and encoded
//!   doubles start at 2^49, above the cage (probe S6).
//! - Stale words in dead frames can keep a dead cell for one more cycle. That is the price of
//!   conservative roots; the stress test counts such cells as "conservatively retained".
//! - Not built under Miri: Miri cannot run the spill trampoline or read raw stack words.

use crate::cell::CellPtr;
use crate::trace::Tracer;

/// Scans `[lo, hi)` and marks every cell a word points into (or one past).
/// Returns the number of words read.
///
/// # Safety
/// `[lo, hi)` is readable memory of the current thread's stack, and `lo` is 8-aligned.
pub(crate) unsafe fn scan_range(
    t: &mut Tracer<'_>,
    lo: usize,
    hi: usize,
    mut log: Option<&mut Vec<CellPtr>>,
) -> usize {
    debug_assert!(lo.is_multiple_of(8) && lo <= hi);
    let mem = t.mem();
    let mut p = lo;
    let mut words = 0;
    while p + 8 <= hi {
        // SAFETY: `p` is 8-aligned and inside the readable range (caller contract).
        // `load_word` is an asm load, so reading uninitialized or borrowed stack slots is
        // not a Rust-level read.
        let w = unsafe { ep9_platform::load_word(p) };
        if mem.in_use_range(w) || mem.in_use_range(w.wrapping_sub(1)) {
            t.mark_conservative(w, log.as_deref_mut());
        }
        p += 8;
        words += 1;
    }
    words
}
