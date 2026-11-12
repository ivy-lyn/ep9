//! The layout table (the H35 S3 outcome of every header and entry byte) and the layout text
//! that `tests/golden/header-layout.txt` holds and `container-history.txt` fingerprints.

use std::fmt::Write as _;

use crate::codes::{Contract, SectionKind, cpu, target_abi, target_arch};
use crate::header::{
    CONTAINER_VERSION, EXTENSION_ALIGN_LOG2, EXTENSION_KIND_MIN, FROZEN_PREFIX_SIZE, HASH_SHA256,
    HEADER_SIZE, MAGIC, MAX_SECTIONS, MIN_SECTIONS, SECTION_ENTRY_SIZE, SECTION_TABLE_OFFSET,
    SOURCE_FLAGS_DEFINED, UNIT_DATA_SLOTS, entry_offsets, offsets,
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
    /// LOAD whatever the value (container v2 compat flags: an unknown bit is ignored).
    Ignored,
    /// LOAD while the value is at most the host's, else BYTECODE (the RuntimeTable length).
    AtMostHost,
    /// LOAD while the value is a unit-data slot (below 64), else BYTECODE.
    Slot,
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
            Self::Ignored => "ignored",
            Self::AtMostHost => "at-most-host",
            Self::Slot => "slot",
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
pub const HEADER_FIELDS: [Field; 35] = {
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
        field("compat_flags", o::COMPAT_FLAGS, 4, Ignored),
        field("incompat_flags", o::INCOMPAT_FLAGS, 4, Bytecode),
        field("vm_layout", o::VM_LAYOUT, 4, Bytecode),
        field("runtime_table_len", o::RUNTIME_TABLE_LEN, 4, AtMostHost),
        field("unit_data_slot", o::UNIT_DATA_SLOT, 1, Slot),
        field("reserved3", o::RESERVED3, 3, LoadError),
        field("root_shapes", o::ROOT_SHAPES, 8, Bytecode),
        field("codegen_flags", o::CODEGEN_FLAGS, 8, Bytecode),
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
         #   table                load-error, unless the table stays canonical (another unit)\n\
         #   ignored              load: an unknown compat flag bit is ignored\n\
         #   at-most-host         load while the value is at most the engine's, else bytecode\n\
         #   slot                 load while the value is a unit-data slot (below 64), else bytecode\n",
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
         {MIN_SECTIONS}..={MAX_SECTIONS} entries, kinds strictly ascending (the named kinds, then \
         the extensions)"
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
            SectionKind::Deps | SectionKind::SnapshotBinding => "reserved: present -> bytecode",
            _ => "optional",
        };
        let _ = writeln!(s, "{} {} {} {rule}", k as u32, k.name(), k.align_log2());
    }
    let _ = writeln!(
        s,
        "extensions: kind >= {EXTENSION_KIND_MIN:#x}, align_log2 {}..={}, in the hash; entry flags \
         bit 0 = native code needs it: an unknown extension with bit 0 -> bytecode, without -> \
         skipped",
        EXTENSION_ALIGN_LOG2.start(),
        EXTENSION_ALIGN_LOG2.end()
    );
    s.push_str(
        "entry flags: bit 0 only on an extension; a named kind has flags 0; other bits are 0\n",
    );
    s.push_str(
        "\nlayout: each section starts at align_up(end of the previous section or of the table, \
         2^align_log2); padding bytes are 0; flags and reserved bytes are 0; the last section \
         ends at total_size. The text is 64 KiB-aligned in the file; a loader maps it in place \
         only from a 64 KiB-aligned address and copies it otherwise\n",
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
    s.push_str(
        "compat_flags: an unknown bit is ignored. incompat_flags: an unknown bit -> bytecode. \
         The defined bits are in the code tables\n",
    );
    let _ = writeln!(
        s,
        "runtime_table_len: the RuntimeTable entries the native code calls; above the engine's \
         -> bytecode. unit_data_slot: 0..{UNIT_DATA_SLOTS} (0 = the engine's own unit), else \
         bytecode; a slot in use in the isolate -> bytecode at load. root_shapes, codegen_flags: \
         8-byte hashes, 0 = none; another value -> bytecode"
    );
    let _ = writeln!(
        s,
        "source_flags: defined mask {SOURCE_FLAGS_DEFINED:#x}; bits 0-1 code kind (0 script, \
         1 module, 2 eval), bit 2 strict, bit 8 typescript, bits 9-11 TS flags only with bit 8"
    );
    let (lines, sha) = CODE_TABLES_V4;
    let _ = writeln!(
        s,
        "code tables (append-only; the H35 Freeze table): the target_arch and target_abi codes, \
         the cpu bits and the compat and incompat bits are the lines of \
         tests/golden/code-tables.txt (<table> <value> <name>), which only append: a value keeps \
         its number and its meaning. A target code that the engine does not know never matches \
         (0 = unknown), and a cpu bit that it does not know counts as missing: bytecode. The \
         first {lines} lines are the tables of container version 4 (SHA-256 {sha})"
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
         (builtin_abi, vm_layout last), incompat_flags, runtime_table_len, root_shapes, \
         codegen_flags, unit_data_slot, target, heap_page_log2, cpu_required, version 0 with a \
         text section, a reserved section, an extension that native code needs\n",
    );
    s
}

/// The lines of the code tables at container version 4 (MC-w29-2; the H35 Freeze-table part
/// "code tables (append-only)"): how many lines `tests/golden/code-tables.txt` held then, and
/// the SHA-256 of those lines (each with its newline). [`layout_text`] prints both, so an edit
/// of one of these lines changes the layout text and needs a container version bump; a line
/// appended after them changes neither.
pub const CODE_TABLES_V4: (usize, &str) = (
    22,
    "ec077908730576ea7073e9de138a490683095c4c3f476c263afab8ba97ff6c27",
);

/// The code tables, one line per defined value, `<table> <value> <name>`: the target
/// architecture and ABI codes, the CPU feature bits (with their architecture) and the
/// `compat_flags` and `incompat_flags` bits (with their meaning). An engine meets an unknown
/// value of each table safely (H35 Freeze table), so a new value is one appended line of
/// `tests/golden/code-tables.txt` and moves no container version (MC-w29-2).
pub fn code_table_lines() -> Vec<String> {
    let mut v = Vec::new();
    for (code, name) in [
        (target_arch::X86_64, "x86_64"),
        (target_arch::AARCH64, "aarch64"),
    ] {
        v.push(format!("target_arch {code} {name}"));
    }
    for (code, name) in [
        (target_abi::LINUX, "linux"),
        (target_abi::APPLE, "apple"),
        (target_abi::WINDOWS, "windows"),
    ] {
        v.push(format!("target_abi {code} {name}"));
    }
    for (bit, name, arch) in cpu::BITS {
        let arch = if arch == target_arch::X86_64 {
            "x86_64"
        } else {
            "aarch64"
        };
        v.push(format!("cpu {} {name} {arch}", bit.trailing_zeros()));
    }
    let compat: [(u32, &str); 1] = [(
        crate::header::compat_flags::HAS_PADS,
        "has_pads: each function entry and each adr target is bti c or bti jc, each br goes \
         through x16 or x17, and each br and blr target is proven a pad or code outside the \
         text: a loader may map the text with BTI",
    )];
    for (bit, text) in compat {
        v.push(format!("compat {} {text}", bit.trailing_zeros()));
    }
    // incompat_flags: no bit defined (an unknown bit gives bytecode).
    v
}
