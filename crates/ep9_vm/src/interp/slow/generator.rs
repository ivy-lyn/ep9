#![allow(dead_code)] // m2-stub
//! Generator opcodes and runtime functions (owner P1-GEN-VM). The three opcodes switch
//! frames or jump, so they use the frame-switch protocol of `slow::call`: the entry sets
//! `cx.out_fp` and `cx.out_pc` (also when it unwinds) and returns the accumulator, and the
//! handler adopts them (`St::switch`).
//!
//! ```text
//! SwitchOnGeneratorState g, table   g undefined: out_pc = next (first call); else the
//!                                   suspend id's table entry, state = executing
//! SuspendGenerator g, regs, n, id   copy params and regs into the generator's REGS cell,
//!                                   state = id, then return like `Return` (caller fp/pc)
//! ResumeGenerator g, regs, n        copy the regs back; acc = the sent value
//! ```
//!
//! Operands: `generator` and `regs` are register slot offsets from `fp` (`dsl::R`,
//! `dsl::RList`), `table` a constant index (a JumpTable). P0 stubs throw "not implemented in
//! ep9 v0: generators", as the opcodes did before. Runtime ids 19-22 (ABI 3) come with the
//! P0-LANG merge.

use crate::error::{self, not_implemented};
use crate::interp::Cx;

use super::enter;

/// The stub body: the v0 "generators" error, then unwind (sets `out_fp`/`out_pc`).
fn not_built(cx: &mut Cx, fp: *mut u64, pc: *const u8) -> u64 {
    enter(cx, fp, pc);
    not_implemented(cx, "generators");
    error::unwind(cx, fp, pc)
}

/// `SwitchOnGeneratorState(generator, table)`. Frame-switch protocol. Stub.
#[cold]
#[inline(never)]
pub(crate) fn switch_on_generator_state(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    _next: *const u8,
    _generator: isize,
    _table: u32,
) -> u64 {
    not_built(cx, fp, pc)
}

/// `SuspendGenerator(generator, registers, count, suspend_id)`; `acc` is the value to yield.
/// Frame-switch protocol. Stub.
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn suspend_generator(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    _next: *const u8,
    _acc: u64,
    _generator: isize,
    _registers: isize,
    _count: u32,
    _suspend_id: u32,
) -> u64 {
    not_built(cx, fp, pc)
}

/// `ResumeGenerator(generator, registers, count)`. Frame-switch protocol. Stub.
#[cold]
#[inline(never)]
pub(crate) fn resume_generator(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    _next: *const u8,
    _generator: isize,
    _registers: isize,
    _count: u32,
) -> u64 {
    not_built(cx, fp, pc)
}
