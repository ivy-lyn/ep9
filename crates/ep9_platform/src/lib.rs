//! OS layer: VA reserve incl. the code range (H7), MAP_JIT and W^X under `jit`, stack bounds,
//! signals and SEH, spill trampolines, and unwind registration for JIT code and loaded units.
//!
//! Built so far (M0 weeks 2-3: SP-VA, SP-GC, SP-JIT-mem):
//!
//! | Module | Contents | Backends | Feature |
//! |---|---|---|---|
//! | [`va`] | [`Reservation`] (reserve, commit, decommit, release), [`Cage`] = heap cage + code range below 2^47 | mmap / VirtualAlloc | always |
//! | [`stack`] | [`current_thread_stack_bounds`], [`stack_pointer`] | pthread / GetCurrentThreadStackLimits | always |
//! | [`spill`] | spill trampolines for AAPCS64, SysV x86-64 and Win64; [`with_spilled_registers`]; [`load_word`] | naked functions | always |
//! | `jit` | `CodeMemory` in the code range with W^X: MAP_JIT, mprotect, dual mapping; icache flush | MAP_JIT / mprotect / memfd / VirtualProtect | `jit` |
//! | `unwind` | `.eh_frame` and Win64 UNWIND_INFO encoders; registration; native stack walk | libgcc / libunwind / RtlAddGrowableFunctionTable | `unwind` (`jit` implies it) |
//!
//! Unit tests build `jit` and `unwind` without the features, so `cargo test --workspace` runs
//! their tests.
//!
//! Not built yet: signals and SEH handlers, unit mapping (A2).

#[cfg(any(feature = "jit", test))]
pub mod jit;
pub mod spill;
pub mod stack;
mod sys;
#[cfg(any(feature = "unwind", test))]
pub mod unwind;
pub mod va;

pub use spill::{
    SpillAbi, SpilledRegisters, load_word, with_spilled_registers, with_spilled_registers_abi,
};
pub use stack::{StackBounds, current_thread_stack_bounds, stack_pointer};
pub use va::{Cage, CageConfig, CageError, Reservation, allocation_granularity, page_size};
