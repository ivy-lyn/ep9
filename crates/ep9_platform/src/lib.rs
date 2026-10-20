//! OS layer: VA reserve incl. the code range (H7), MAP_JIT and W^X under `jit`, stack bounds,
//! signals and SEH, spill trampolines, and unwind registration for JIT code and loaded units.
//!
//! Built so far (M0 week 2, SP-VA and SP-GC):
//!
//! | Module | Contents | Backends |
//! |---|---|---|
//! | [`va`] | [`Reservation`] (reserve, commit, decommit, release), [`Cage`] = heap cage + code range below 2^47 | mmap / VirtualAlloc |
//! | [`stack`] | [`current_thread_stack_bounds`], [`stack_pointer`] | pthread / GetCurrentThreadStackLimits |
//! | [`spill`] | spill trampolines for AAPCS64, SysV x86-64 and Win64; [`with_spilled_registers`]; [`load_word`] | naked functions |
//!
//! Not built yet: MAP_JIT and W^X (`jit`, SP-JIT-mem), signals and SEH handlers, unwind
//! registration.

pub mod spill;
pub mod stack;
mod sys;
pub mod va;

pub use spill::{
    SpillAbi, SpilledRegisters, load_word, with_spilled_registers, with_spilled_registers_abi,
};
pub use stack::{StackBounds, current_thread_stack_bounds, stack_pointer};
pub use va::{Cage, CageConfig, CageError, Reservation, allocation_granularity, page_size};
