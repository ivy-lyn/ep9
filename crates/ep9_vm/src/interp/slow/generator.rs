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
//! ep9 v0: generators", as the opcodes did before.
//!
//! Runtime ids 19-22 (ABI 3, `slow/runtime.rs` routes them here) use the result protocol of
//! `CallRuntime` (`Res<u64>`, the new accumulator). P0 stubs throw
//! `not implemented in ep9 v0: <name>`, the text of the catch-all arm before they were routed.

use crate::conv::Res;
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

// ---- runtime ids 19-22 (ABI 3) -------------------------------------------------------------

/// `CreateGeneratorObject(function, receiver)` (runtime id 19): the generator object of the
/// running generator (or async) function `function` in frame `fp`, with `receiver` as its
/// `this`; the context and the register count come from the frame. Stub.
pub(crate) fn create_generator_object(
    cx: &mut Cx,
    _fp: *mut u64,
    _function: u64,
    _receiver: u64,
) -> Res<u64> {
    Err(not_implemented(cx, "CreateGeneratorObject"))
}

/// `GeneratorGetResumeMode(generator)` (runtime id 20): next, return or throw, as a Smi. Stub.
pub(crate) fn generator_get_resume_mode(cx: &mut Cx, _generator: u64) -> Res<u64> {
    Err(not_implemented(cx, "GeneratorGetResumeMode"))
}

/// `GeneratorClose(generator)` (runtime id 21): the generator is completed. Stub.
pub(crate) fn generator_close(cx: &mut Cx, _generator: u64) -> Res<u64> {
    Err(not_implemented(cx, "GeneratorClose"))
}

/// `ThrowThrowMethodMissing()` (runtime id 22): the TypeError of `yield*` when the inner
/// iterator has no `throw` method (§15.5.5 step 7.b.iii). Stub.
pub(crate) fn throw_throw_method_missing(cx: &mut Cx) -> Res<u64> {
    Err(not_implemented(cx, "ThrowThrowMethodMissing"))
}
