//! Codes the header uses: contracts and their versions, target codes, CPU feature bits and
//! section kinds.

use crate::header::offsets;

/// A contract whose version the header records (H35-Q4: one version per contract).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Contract {
    /// Bytecode ABI (H2). A mismatch gives SOURCE.
    BytecodeAbi,
    /// Value encoding (H6).
    ValueEncoding,
    /// Object layout (H8, H17).
    ObjectLayout,
    /// Store barrier (H11).
    Barrier,
    /// Frame layout (H14).
    Frame,
    /// IC and feedback layout (H16).
    Ic,
    /// Deopt metadata (H29).
    DeoptMeta,
    /// FactSnapshot schema (H16, H37). A mismatch gives BYTECODE and drops the hints section.
    FactSchema,
    /// Builtin ABI (H20).
    BuiltinAbi,
}

impl Contract {
    /// Every contract, in check order (header order, with the builtin ABI last).
    pub const ALL: [Contract; 9] = [
        Self::BytecodeAbi,
        Self::ValueEncoding,
        Self::ObjectLayout,
        Self::Barrier,
        Self::Frame,
        Self::Ic,
        Self::DeoptMeta,
        Self::FactSchema,
        Self::BuiltinAbi,
    ];

    /// The field name in the layout text.
    pub const fn name(self) -> &'static str {
        match self {
            Self::BytecodeAbi => "bytecode_abi",
            Self::ValueEncoding => "value_encoding",
            Self::ObjectLayout => "object_layout",
            Self::Barrier => "barrier",
            Self::Frame => "frame",
            Self::Ic => "ic",
            Self::DeoptMeta => "deopt_meta",
            Self::FactSchema => "fact_schema",
            Self::BuiltinAbi => "builtin_abi",
        }
    }

    /// The header offset of this contract's version.
    pub const fn offset(self) -> usize {
        match self {
            Self::BytecodeAbi => offsets::BYTECODE_ABI,
            Self::ValueEncoding => offsets::VALUE_ENCODING,
            Self::ObjectLayout => offsets::OBJECT_LAYOUT,
            Self::Barrier => offsets::BARRIER,
            Self::Frame => offsets::FRAME,
            Self::Ic => offsets::IC,
            Self::DeoptMeta => offsets::DEOPT_META,
            Self::FactSchema => offsets::FACT_SCHEMA,
            Self::BuiltinAbi => offsets::BUILTIN_ABI,
        }
    }

    /// Native code bakes this contract in, so version 0 means "no native form yet": a unit
    /// with a text section runs its bytecode while any native contract is at version 0.
    pub const fn is_native(self) -> bool {
        !matches!(self, Self::BytecodeAbi | Self::FactSchema)
    }
}

/// The contract versions of a unit or a host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Versions {
    /// Bytecode ABI (`ep9_bytecode::ABI_VERSION`).
    pub bytecode_abi: u32,
    /// Value encoding (`ep9_value::encoding::VERSION`).
    pub value_encoding: u32,
    /// Object layout (`ep9_vm::versions::OBJECT_LAYOUT_VERSION`).
    pub object_layout: u32,
    /// Barrier (`ep9_heap::BARRIER_VERSION`).
    pub barrier: u32,
    /// Frame (`ep9_vm::versions::FRAME_VERSION`).
    pub frame: u32,
    /// IC and feedback layout (`ep9_vm::versions::IC_LAYOUT_VERSION`).
    pub ic: u32,
    /// Deopt metadata (`ep9_vm::versions::DEOPT_META_VERSION`).
    pub deopt_meta: u32,
    /// FactSnapshot schema (`ep9_facts::FACT_SCHEMA_VERSION`).
    pub fact_schema: u32,
    /// Builtin ABI (`ep9_vm::versions::BUILTIN_ABI_VERSION`).
    pub builtin_abi: u32,
}

impl Versions {
    /// The version of `c`.
    pub const fn get(&self, c: Contract) -> u32 {
        match c {
            Contract::BytecodeAbi => self.bytecode_abi,
            Contract::ValueEncoding => self.value_encoding,
            Contract::ObjectLayout => self.object_layout,
            Contract::Barrier => self.barrier,
            Contract::Frame => self.frame,
            Contract::Ic => self.ic,
            Contract::DeoptMeta => self.deopt_meta,
            Contract::FactSchema => self.fact_schema,
            Contract::BuiltinAbi => self.builtin_abi,
        }
    }

    /// Sets the version of `c`.
    pub fn set(&mut self, c: Contract, v: u32) {
        *match c {
            Contract::BytecodeAbi => &mut self.bytecode_abi,
            Contract::ValueEncoding => &mut self.value_encoding,
            Contract::ObjectLayout => &mut self.object_layout,
            Contract::Barrier => &mut self.barrier,
            Contract::Frame => &mut self.frame,
            Contract::Ic => &mut self.ic,
            Contract::DeoptMeta => &mut self.deopt_meta,
            Contract::FactSchema => &mut self.fact_schema,
            Contract::BuiltinAbi => &mut self.builtin_abi,
        } = v;
    }
}

/// Target architecture codes (offset 120). Code 0 is "unknown" and never matches.
pub mod target_arch {
    /// x86-64.
    pub const X86_64: u8 = 1;
    /// AArch64.
    pub const AARCH64: u8 = 2;
}

/// Target ABI codes (offset 121): object format, calling convention and unwind format. Code 0
/// is "unknown" and never matches.
pub mod target_abi {
    /// Linux: ELF, System V x86-64 or AAPCS64, DWARF CFI.
    pub const LINUX: u8 = 1;
    /// Apple: Mach-O, Darwin ABI, compact unwind.
    pub const APPLE: u8 = 2;
    /// Windows: COFF, Win64 ABI, SEH unwind tables.
    pub const WINDOWS: u8 = 3;
}

/// Required-CPU-feature bits (offset 128, H35-Q5). x86-64 features use bits 0-31 and AArch64
/// features bits 32-63, so a unit for one architecture never requires a feature of the other.
/// The host must have every required bit; a bit not listed here counts as missing.
pub mod cpu {
    /// x86-64 SSE3.
    pub const X86_SSE3: u64 = 1 << 0;
    /// x86-64 SSSE3.
    pub const X86_SSSE3: u64 = 1 << 1;
    /// x86-64 SSE4.1.
    pub const X86_SSE4_1: u64 = 1 << 2;
    /// x86-64 SSE4.2.
    pub const X86_SSE4_2: u64 = 1 << 3;
    /// x86-64 POPCNT.
    pub const X86_POPCNT: u64 = 1 << 4;
    /// x86-64 AVX.
    pub const X86_AVX: u64 = 1 << 5;
    /// x86-64 AVX2.
    pub const X86_AVX2: u64 = 1 << 6;
    /// x86-64 FMA3.
    pub const X86_FMA: u64 = 1 << 7;
    /// x86-64 BMI1.
    pub const X86_BMI1: u64 = 1 << 8;
    /// x86-64 BMI2.
    pub const X86_BMI2: u64 = 1 << 9;
    /// x86-64 LZCNT.
    pub const X86_LZCNT: u64 = 1 << 10;
    /// AArch64 LSE atomics.
    pub const A64_LSE: u64 = 1 << 32;
    /// AArch64 FJCVTZS (JavaScript double to int32 conversion).
    pub const A64_JSCONV: u64 = 1 << 33;
    /// AArch64 dot product.
    pub const A64_DOTPROD: u64 = 1 << 34;
    /// AArch64 half-precision arithmetic.
    pub const A64_FP16: u64 = 1 << 35;
    /// AArch64 RCpc load-acquire.
    pub const A64_RCPC: u64 = 1 << 36;

    /// Every defined bit: (bit, name, architecture code). The names are the
    /// `is_x86_feature_detected!` / `is_aarch64_feature_detected!` names.
    pub const BITS: [(u64, &str, u8); 16] = [
        (X86_SSE3, "sse3", super::target_arch::X86_64),
        (X86_SSSE3, "ssse3", super::target_arch::X86_64),
        (X86_SSE4_1, "sse4.1", super::target_arch::X86_64),
        (X86_SSE4_2, "sse4.2", super::target_arch::X86_64),
        (X86_POPCNT, "popcnt", super::target_arch::X86_64),
        (X86_AVX, "avx", super::target_arch::X86_64),
        (X86_AVX2, "avx2", super::target_arch::X86_64),
        (X86_FMA, "fma", super::target_arch::X86_64),
        (X86_BMI1, "bmi1", super::target_arch::X86_64),
        (X86_BMI2, "bmi2", super::target_arch::X86_64),
        (X86_LZCNT, "lzcnt", super::target_arch::X86_64),
        (A64_LSE, "lse", super::target_arch::AARCH64),
        (A64_JSCONV, "jsconv", super::target_arch::AARCH64),
        (A64_DOTPROD, "dotprod", super::target_arch::AARCH64),
        (A64_FP16, "fp16", super::target_arch::AARCH64),
        (A64_RCPC, "rcpc", super::target_arch::AARCH64),
    ];

    /// The union of [`BITS`].
    pub const DEFINED: u64 = {
        let mut m = 0;
        let mut i = 0;
        while i < BITS.len() {
            m |= BITS[i].0;
            i += 1;
        }
        m
    };
}

/// Section kinds (H35 S2). The table lists each kind at most once, in ascending order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u32)]
pub enum SectionKind {
    /// Native code (baseline-AOT, optimized-AOT), RX, shared per process.
    Text = 1,
    /// Deopt translations, safepoint and pc-to-bytecode maps, function table.
    Rodata = 2,
    /// Link table and IC cells, copied per isolate.
    RwTemplate = 3,
    /// The CompiledUnit (H2). Required.
    Bytecode = 4,
    /// The source text (H5). Required.
    Source = 5,
    /// A HINTED FactSnapshot (`ep9_facts` format). Optional.
    Hints = 6,
    /// Unwind tables.
    Unwind = 7,
    /// Debug data, perf map.
    Debug = 8,
    /// Signature over the content hash (H35-Q2, D6). Kept out of the hash.
    Signature = 9,
}

impl SectionKind {
    /// Every kind, in table order.
    pub const ALL: [SectionKind; 9] = [
        Self::Text,
        Self::Rodata,
        Self::RwTemplate,
        Self::Bytecode,
        Self::Source,
        Self::Hints,
        Self::Unwind,
        Self::Debug,
        Self::Signature,
    ];

    /// The kind with id `v`.
    pub const fn from_u32(v: u32) -> Option<SectionKind> {
        match v {
            1 => Some(Self::Text),
            2 => Some(Self::Rodata),
            3 => Some(Self::RwTemplate),
            4 => Some(Self::Bytecode),
            5 => Some(Self::Source),
            6 => Some(Self::Hints),
            7 => Some(Self::Unwind),
            8 => Some(Self::Debug),
            9 => Some(Self::Signature),
            _ => None,
        }
    }

    /// The name in the layout text.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Rodata => "rodata",
            Self::RwTemplate => "rw_template",
            Self::Bytecode => "bytecode",
            Self::Source => "source",
            Self::Hints => "hints",
            Self::Unwind => "unwind",
            Self::Debug => "debug",
            Self::Signature => "signature",
        }
    }

    /// log2 of the section's alignment in the file. Container version 1 aligns every section
    /// to 8 bytes; the A2 loader may raise the text alignment with a container version bump.
    pub const fn align_log2(self) -> u8 {
        3
    }

    /// A unit without this section is a load error. The source is always present (H5); the
    /// BYTECODE outcome and every deopt run the bytecode.
    pub const fn required(self) -> bool {
        matches!(self, Self::Bytecode | Self::Source)
    }
}
