//! What the running engine offers a unit: build id, contract versions, target, page size and
//! CPU features. [`crate::check()`] compares a unit header with it.

use crate::codes::{Versions, cpu, target_abi, target_arch};

/// The engine build id this build writes and accepts (H35-Q8). All zero until SP-Unit picks
/// its source, the same value `ep9_frontend` writes into every CompiledUnit today.
pub const ENGINE_BUILD_ID: [u8; 16] = [0; 16];

/// The host side of the check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostInfo {
    /// Engine build id.
    pub engine_build_id: [u8; 16],
    /// Contract versions of this build.
    pub versions: Versions,
    /// [`target_arch`] code; 0 if unknown.
    pub target_arch: u8,
    /// [`target_abi`] code; 0 if unknown.
    pub target_abi: u8,
    /// log2 of the heap page size.
    pub heap_page_log2: u8,
    /// [`cpu`] bits this CPU has.
    pub cpu_features: u64,
}

impl Versions {
    /// The versions this build was compiled with.
    pub fn current() -> Versions {
        use ep9_vm::versions as vm;
        Versions {
            bytecode_abi: ep9_bytecode::ABI_VERSION,
            value_encoding: ep9_value::encoding::VERSION,
            object_layout: vm::OBJECT_LAYOUT_VERSION,
            barrier: ep9_heap::BARRIER_VERSION,
            frame: vm::FRAME_VERSION,
            ic: vm::IC_LAYOUT_VERSION,
            deopt_meta: vm::DEOPT_META_VERSION,
            fact_schema: ep9_facts::FACT_SCHEMA_VERSION,
            builtin_abi: vm::BUILTIN_ABI_VERSION,
        }
    }
}

impl HostInfo {
    /// This build on this CPU: the version constants of every contract crate, the page size
    /// of `ep9_heap`, the compile target and the CPU features detected now.
    pub fn current() -> HostInfo {
        HostInfo {
            engine_build_id: ENGINE_BUILD_ID,
            versions: Versions::current(),
            target_arch: current_arch(),
            target_abi: current_abi(),
            heap_page_log2: ep9_heap::layout::PAGE_SIZE.trailing_zeros() as u8,
            cpu_features: detect_cpu(),
        }
    }
}

const fn current_arch() -> u8 {
    if cfg!(target_arch = "x86_64") {
        target_arch::X86_64
    } else if cfg!(target_arch = "aarch64") {
        target_arch::AARCH64
    } else {
        0
    }
}

const fn current_abi() -> u8 {
    if cfg!(target_vendor = "apple") {
        target_abi::APPLE
    } else if cfg!(target_os = "windows") {
        target_abi::WINDOWS
    } else if cfg!(target_os = "linux") {
        target_abi::LINUX
    } else {
        0
    }
}

#[cfg(target_arch = "x86_64")]
fn detect_cpu() -> u64 {
    use std::arch::is_x86_feature_detected as has;
    let found = [
        (cpu::X86_SSE3, has!("sse3")),
        (cpu::X86_SSSE3, has!("ssse3")),
        (cpu::X86_SSE4_1, has!("sse4.1")),
        (cpu::X86_SSE4_2, has!("sse4.2")),
        (cpu::X86_POPCNT, has!("popcnt")),
        (cpu::X86_AVX, has!("avx")),
        (cpu::X86_AVX2, has!("avx2")),
        (cpu::X86_FMA, has!("fma")),
        (cpu::X86_BMI1, has!("bmi1")),
        (cpu::X86_BMI2, has!("bmi2")),
        (cpu::X86_LZCNT, has!("lzcnt")),
    ];
    found
        .iter()
        .filter(|(_, on)| *on)
        .fold(0, |m, (b, _)| m | b)
}

#[cfg(target_arch = "aarch64")]
fn detect_cpu() -> u64 {
    use std::arch::is_aarch64_feature_detected as has;
    let found = [
        (cpu::A64_LSE, has!("lse")),
        (cpu::A64_JSCONV, has!("jsconv")),
        (cpu::A64_DOTPROD, has!("dotprod")),
        (cpu::A64_FP16, has!("fp16")),
        (cpu::A64_RCPC, has!("rcpc")),
    ];
    found
        .iter()
        .filter(|(_, on)| *on)
        .fold(0, |m, (b, _)| m | b)
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn detect_cpu() -> u64 {
    0
}
