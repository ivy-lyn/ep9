//! The layout table (the H35 S3 outcome of every header and entry byte) and the layout text
//! that `tests/golden/header-layout.txt` holds and `container-history.txt` fingerprints.

use std::fmt::Write as _;

use crate::codes::{Contract, SectionKind, cpu, target_abi, target_arch};
use crate::header::{
    CONTAINER_VERSION, FROZEN_PREFIX_SIZE, HASH_SHA256, HEADER_SIZE, MAGIC, MAX_SECTIONS,
    MIN_SECTIONS, SECTION_ENTRY_SIZE, SECTION_TABLE_OFFSET, SOURCE_FLAGS_DEFINED, entry_offsets,
    offsets,
};

/// What [`crate::check()`] gives when one field differs from a unit that loads and the content
/// hash is recomputed (a unit written that way). Without the recompute, every change is a LOAD
/// ERROR (S4 step 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnChange {
    /// S4 step 1 or 2: LOAD ERROR.
    LoadError,
    /// S4 step 3: SOURCE.
    Source,
    /// S4 step 4: BYTECODE.
    Bytecode,
    /// S4 step 4: BYTECODE, and the hints section is dropped.
    BytecodeDropHints,
    /// LOAD while every required bit is a host feature, else BYTECODE.
    CpuSubset,
    /// LOAD while [`crate::source_flags_valid`], else LOAD ERROR.
    SourceFlags,
    /// LOAD ERROR, unless the table is still canonical: then it is another well-formed unit.
    Table,
}

impl OnChange {
    /// The name in the layout text.
    pub const fn name(self) -> &'static str {
        match self {
            Self::LoadError => "load-error",
            Self::Source => "source",
            Self::Bytecode => "bytecode",
            Self::BytecodeDropHints => "bytecode-drop-hints",
            Self::CpuSubset => "cpu-subset",
            Self::SourceFlags => "source-flags",
            Self::Table => "table",
        }
    }
}

/// One field of the layout table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    /// Name.
    pub name: &'static str,
    /// Byte offset.
    pub offset: usize,
    /// Size in bytes.
    pub size: usize,
    /// The outcome of a change.
    pub on_change: OnChange,
}

const fn field(name: &'static str, offset: usize, size: usize, on_change: OnChange) -> Field {
    Field {
        name,
        offset,
        size,
        on_change,
    }
}

/// Every header byte, in order (the H35 S3 table, as built).
pub const HEADER_FIELDS: [Field; 27] = {
    use OnChange::*;
    use offsets as o;
    [
        field("magic", o::MAGIC, 8, LoadError),
        field("container_version", o::CONTAINER_VERSION, 4, LoadError),
        field("header_size", o::HEADER_SIZE, 4, LoadError),
        field("total_size", o::TOTAL_SIZE, 8, LoadError),
        field("section_count", o::SECTION_COUNT, 4, LoadError),
        field(
            "section_table_offset",
            o::SECTION_TABLE_OFFSET,
            4,
            LoadError,
        ),
        field("hash_alg", o::HASH_ALG, 1, LoadError),
        field("header_flags", o::HEADER_FLAGS, 1, LoadError),
        field("reserved0", o::RESERVED0, 6, LoadError),
        field("content_hash", o::CONTENT_HASH, 32, LoadError),
        field("engine_build_id", o::ENGINE_BUILD_ID, 16, Source),
        field("bytecode_abi", o::BYTECODE_ABI, 4, Source),
        field("value_encoding", o::VALUE_ENCODING, 4, Bytecode),
        field("object_layout", o::OBJECT_LAYOUT, 4, Bytecode),
        field("barrier", o::BARRIER, 4, Bytecode),
        field("frame", o::FRAME, 4, Bytecode),
        field("ic", o::IC, 4, Bytecode),
        field("deopt_meta", o::DEOPT_META, 4, Bytecode),
        field("fact_schema", o::FACT_SCHEMA, 4, BytecodeDropHints),
        field("target_arch", o::TARGET_ARCH, 1, Bytecode),
        field("target_abi", o::TARGET_ABI, 1, Bytecode),
        field("heap_page_log2", o::HEAP_PAGE_LOG2, 1, Bytecode),
        field("reserved1", o::RESERVED1, 1, LoadError),
        field("source_flags", o::SOURCE_FLAGS, 4, SourceFlags),
        field("cpu_required", o::CPU_REQUIRED, 8, CpuSubset),
        field("builtin_abi", o::BUILTIN_ABI, 4, Bytecode),
        field(
            "reserved2",
            o::RESERVED2,
            HEADER_SIZE - o::RESERVED2,
            LoadError,
        ),
    ]
};

/// Every byte of a section table entry, in order.
pub const ENTRY_FIELDS: [Field; 6] = {
    use OnChange::Table;
    use entry_offsets as o;
    [
        field("kind", o::KIND, 4, Table),
        field("flags", o::FLAGS, 4, Table),
        field("offset", o::OFFSET, 8, Table),
        field("size", o::SIZE, 8, Table),
        field("align_log2", o::ALIGN_LOG2, 1, Table),
        field(
            "reserved",
            o::RESERVED,
            SECTION_ENTRY_SIZE - o::RESERVED,
            Table,
        ),
    ]
};

/// The container contract as text: header fields, entry fields, section kinds, layout and hash
/// rules, codes and CPU bits. `tests/golden/header-layout.txt` holds it, and
/// `tests/golden/container-history.txt` pins its SHA-256 for [`CONTAINER_VERSION`].
pub fn layout_text() -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "# ep9 unit container, container_version {CONTAINER_VERSION} \
         (docs/adr/H35-aot-unit-format.md)."
    );
    s.push_str(
        "# Generated by ep9_unit::layout_text(). A change here needs a CONTAINER_VERSION bump.\n\
         # on-change: what check() gives when only this field differs from a unit that loads and\n\
         # the content hash is recomputed. Without the recompute every change is load-error.\n\
         #   load-error           S4 step 1 or 2: clean load error, no section used\n\
         #   source               S4 step 3: compile the source section\n\
         #   bytecode             S4 step 4: run the bytecode, drop native code\n\
         #   bytecode-drop-hints  S4 step 4: as bytecode, and drop the hints section\n\
         #   cpu-subset           load while every required bit is a host feature, else bytecode\n\
         #   source-flags         load while the flags are valid (rule below), else load-error\n\
         #   table                load-error, unless the table stays canonical (another unit)\n",
    );
    let _ = writeln!(
        s,
        "\nheader: {HEADER_SIZE} bytes, little-endian, frozen prefix 0..{FROZEN_PREFIX_SIZE}"
    );
    s.push_str("offset size field on-change\n");
    for f in &HEADER_FIELDS {
        let _ = writeln!(
            s,
            "{} {} {} {}",
            f.offset,
            f.size,
            f.name,
            f.on_change.name()
        );
    }
    let _ = writeln!(
        s,
        "\nsection entry: {SECTION_ENTRY_SIZE} bytes; table at offset {SECTION_TABLE_OFFSET}, \
         {MIN_SECTIONS}..={MAX_SECTIONS} entries, kinds strictly ascending"
    );
    s.push_str("offset size field on-change\n");
    for f in &ENTRY_FIELDS {
        let _ = writeln!(
            s,
            "{} {} {} {}",
            f.offset,
            f.size,
            f.name,
            f.on_change.name()
        );
    }
    s.push_str("\nsection kinds: id name align_log2 rule\n");
    for k in SectionKind::ALL {
        let rule = match k {
            SectionKind::Text => "native code: bytecode while any native contract version is 0",
            SectionKind::Bytecode | SectionKind::Source => "required",
            SectionKind::Signature => "present iff header_flags bit 0; outside the hash",
            _ => "optional",
        };
        let _ = writeln!(s, "{} {} {} {rule}", k as u32, k.name(), k.align_log2());
    }
    s.push_str(
        "\nlayout: each section starts at align_up(end of the previous section or of the table, \
         2^align_log2); padding bytes are 0; flags and reserved bytes are 0; the last section \
         ends at total_size\n",
    );
    s.push_str(
        "hash: hash_alg 1 = SHA-256 of the whole unit with content_hash and the signature \
         section's bytes set to 0\n",
    );
    let magic: Vec<String> = MAGIC.iter().map(|b| format!("{b:02x}")).collect();
    let _ = writeln!(s, "magic: {}", magic.join(" "));
    let _ = writeln!(
        s,
        "fixed values: container_version {CONTAINER_VERSION}, header_size {HEADER_SIZE}, \
         section_table_offset {SECTION_TABLE_OFFSET}, hash_alg {HASH_SHA256}"
    );
    let _ = writeln!(s, "header_flags: bit 0 signed");
    let _ = writeln!(
        s,
        "source_flags: defined mask {SOURCE_FLAGS_DEFINED:#x}; bits 0-1 code kind (0 script, \
         1 module, 2 eval), bit 2 strict, bit 8 typescript, bits 9-11 TS flags only with bit 8"
    );
    let _ = writeln!(
        s,
        "target_arch: 0 unknown (never matches), {} x86_64, {} aarch64",
        target_arch::X86_64,
        target_arch::AARCH64
    );
    let _ = writeln!(
        s,
        "target_abi: 0 unknown (never matches), {} linux, {} apple, {} windows",
        target_abi::LINUX,
        target_abi::APPLE,
        target_abi::WINDOWS
    );
    let native: Vec<&str> = Contract::ALL
        .iter()
        .filter(|c| c.is_native())
        .map(|c| c.name())
        .collect();
    let _ = writeln!(
        s,
        "native contracts (version 0 = no native form): {}",
        native.join(" ")
    );
    s.push_str(
        "check order: step 1 header, total_size, table, required sections, signature flag; \
         step 2 hash; step 3 engine_build_id, bytecode_abi; step 4 versions in header order \
         (builtin_abi last), target, heap_page_log2, cpu_required, then version 0 with a text \
         section\n",
    );
    s.push_str("cpu bits: bit name arch (a bit not listed counts as missing on every host)\n");
    for (bit, name, arch) in cpu::BITS {
        let arch = if arch == target_arch::X86_64 {
            "x86_64"
        } else {
            "aarch64"
        };
        let _ = writeln!(s, "{} {name} {arch}", bit.trailing_zeros());
    }
    s
}
