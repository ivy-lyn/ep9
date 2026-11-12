//! OS layer: VA reserve incl. the code range (H7), MAP_JIT and W^X under `jit`, stack bounds,
//! signals and SEH, spill trampolines, and unwind registration for JIT code and loaded units.
//!
//! Built so far (M0 weeks 2-3: SP-VA, SP-GC, SP-JIT-mem):
//!
//! | Module | Contents | Backends | Feature |
//! |---|---|---|---|
//! | [`va`] | [`Reservation`] (reserve, commit, decommit, release), [`Cage`] = one heap cage below 2^47, [`CodeRange::process`] = the one code range of the process (128 MiB below 2^47; `MAP_JIT` on macOS arm64 with `jit`) handing out [`CodeChunk`]s | mmap / VirtualAlloc | always |
//! | [`stack`] | [`current_thread_stack_bounds`], [`stack_pointer`] | pthread / GetCurrentThreadStackLimits | always |
//! | [`time`] | [`time::now_epoch_ns`] (the wall clock), [`time::local_offset_ms`] (LocalTZA for Date, §21.4.1.25) | SystemTime; localtime_r / `_localtime64_s` | always |
//! | [`spill`] | spill trampolines for AAPCS64, SysV x86-64 and Win64; [`with_spilled_registers`]; [`load_word`] | naked functions | always |
//! | `jit` | `CodeMemory` in a code-range chunk (`CodeChunk::commit`) with W^X: MAP_JIT, mprotect, dual mapping; icache flush | MAP_JIT / mprotect / memfd / VirtualProtect | `jit` |
//! | `unwind` | `.eh_frame` and Win64 UNWIND_INFO encoders; registration; native stack walk | libgcc / libunwind / RtlAddGrowableFunctionTable | `unwind` (`jit` implies it) |
//! | [`unit_text`] | [`unit_text::UnitText`]: unit text in a code-range chunk, written once, then RX (A2, M7-P4); with `jit` or a `MAP_JIT` range through a `jit` write window, else commit RW, copy, icache flush, protect RX; or a linked text in place ([`unit_text::UnitText::linked`]) | mprotect / VirtualProtect | always |
//! | [`linked`] | static registration of a linked `.o` unit (H35 S1, H35-Q3; M8-P3): [`linked::LinkedUnit`], the record the `ep9_unit` writer emits; [`linked::register`]; [`linked::linked_text`] | the static linker | always |
//!
//! Unit tests build `jit` and `unwind` without the features, so `cargo test --workspace` runs
//! their tests.
//!
//! Not built yet: signals and SEH handlers.

#[cfg(any(feature = "jit", test))]
pub mod jit;
pub mod linked;
pub mod spill;
pub mod stack;
mod sys;
pub mod time;
pub mod unit_text;
#[cfg(any(feature = "unwind", test))]
pub mod unwind;
pub mod va;

pub use spill::{
    SpillAbi, SpilledRegisters, load_word, with_spilled_registers, with_spilled_registers_abi,
};
pub use stack::{StackBounds, current_thread_stack_bounds, stack_pointer};
pub use va::{
    Cage, CageConfig, CageError, CodeChunk, CodeRange, CodeRangeConfig, Reservation,
    allocation_granularity, page_size,
};
