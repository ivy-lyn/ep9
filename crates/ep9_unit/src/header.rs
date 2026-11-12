//! Unit header v2 (H35-Q1; container v2, doc 13 C-16): 256 bytes, little-endian, fixed offsets,
//! 8-byte aligned. A section table follows it, then the sections.
//!
//! ```text
//!   0 magic "\x89EP9UN\r\n"      8 container_version u32 (3)    12 header_size u32 (256)
//!  16 total_size u64            24 section_count u32           28 section_table_offset u32 (256)
//!     -- 32-byte frozen prefix: the same place and meaning in every container version --
//!  32 hash_alg u8 (1 = SHA-256)  33 header_flags u8 (bit 0 signed)  34 reserved [6] = 0
//!  40 content_hash [32]         72 engine_build_id [16]
//!  88 bytecode_abi  92 value_encoding  96 object_layout  100 barrier  104 frame  108 ic
//! 112 deopt_meta   116 fact_schema                                          (u32 each)
//! 120 target_arch u8  121 target_abi u8  122 heap_page_log2 u8  123 reserved u8 = 0
//! 124 source_flags u32 (ep9_bytecode unit flag bits)  128 cpu_required u64
//! 136 builtin_abi u32  140 compat_flags u32  144 incompat_flags u32  148 vm_layout u32
//! 152 runtime_table_len u32  156 unit_data_slot u8  157 reserved [3] = 0  160 root_shapes [8]
//! 168 codegen_flags [8]  176..256 reserved = 0
//! section entry (32 B at 256 + 32 i; kinds strictly ascending; 1-11 named, >= 0x100 extensions):
//!   0 kind u32  4 flags u32 (bit 0: native code needs it)  8 offset u64  16 size u64  24 align_log2 u8  25 reserved [7]
//! ```
//! The header holds sizes, offsets, versions and hashes, never an address (H12). The layout is
//! canonical: a unit's content has exactly one encoding. [`crate::layout_text`] prints the
//! whole contract; `tests/golden/container-history.txt` pins its SHA-256 per container version.

use core::fmt;

use ep9_bytecode::{CodeKind, unit_flags};

use crate::codes::{Contract, SectionKind, Versions};

/// Magic bytes: a high byte (catches 7-bit transfers), "EP9UN", CR LF (catches line-ending
/// conversion), as in the PNG signature and the CompiledUnit magic.
pub const MAGIC: [u8; 8] = [0x89, b'E', b'P', b'9', b'U', b'N', 0x0D, 0x0A];
/// Container version. Any change to [`crate::layout_text`] bumps it (`tests/header.rs`
/// enforces this).
pub const CONTAINER_VERSION: u32 = 3;
/// Header size in bytes.
pub const HEADER_SIZE: usize = 256;
/// Size of the frozen prefix: magic, container version, header size, total size and the
/// section table location.
pub const FROZEN_PREFIX_SIZE: usize = 32;
/// Size of one section table entry.
pub const SECTION_ENTRY_SIZE: usize = 32;
/// Where the section table starts: right after the header.
pub const SECTION_TABLE_OFFSET: usize = HEADER_SIZE;
/// Hash algorithm id of SHA-256, the only one in container version 1.
pub const HASH_SHA256: u8 = 1;
/// The fewest sections a unit has: bytecode and source.
pub const MIN_SECTIONS: u32 = 2;
/// The most sections a unit has: the named kinds at most once each, and extension sections.
pub const MAX_SECTIONS: u32 = 64;

/// Header flag bits (offset 33).
pub mod header_flags {
    /// Bit 0: the unit has a signature section. v0 defines no signature scheme (H35-Q2); the
    /// section is carried, kept out of the hash and not checked (D6: no sandbox in v1).
    pub const SIGNED: u8 = 1;
    /// Every defined bit.
    pub const DEFINED: u8 = SIGNED;
}

/// The CompiledUnit flag bits that the source flags field may hold (offset 124): the goal,
/// strict and TS flags the SOURCE path compiles with (H35 S4). The section bits of a
/// CompiledUnit (`HAS_SOURCE`, `HAS_HINTS`) are not part of it.
pub const SOURCE_FLAGS_DEFINED: u32 =
    unit_flags::CODE_KIND_MASK | unit_flags::STRICT | unit_flags::TYPESCRIPT | TS_FLAGS;

const TS_FLAGS: u32 = unit_flags::TS_USE_DEFINE_FOR_CLASS_FIELDS
    | unit_flags::TS_EXPERIMENTAL_DECORATORS
    | unit_flags::TS_EMIT_DECORATOR_METADATA;

/// Whether `f` is a valid source flags value: only defined bits, a known code kind, and TS
/// flags only on a TypeScript unit (the same rules as a CompiledUnit header).
pub const fn source_flags_valid(f: u32) -> bool {
    f & !SOURCE_FLAGS_DEFINED == 0
        && CodeKind::from_u8((f & unit_flags::CODE_KIND_MASK) as u8).is_some()
        && (f & unit_flags::TYPESCRIPT != 0 || f & TS_FLAGS == 0)
}

/// Byte offsets of the header fields.
pub mod offsets {
    /// Magic, 8 bytes.
    pub const MAGIC: usize = 0;
    /// Container version, u32.
    pub const CONTAINER_VERSION: usize = 8;
    /// Header size, u32.
    pub const HEADER_SIZE: usize = 12;
    /// Total unit size, u64.
    pub const TOTAL_SIZE: usize = 16;
    /// Number of section table entries, u32.
    pub const SECTION_COUNT: usize = 24;
    /// Section table offset, u32.
    pub const SECTION_TABLE_OFFSET: usize = 28;
    /// Hash algorithm, u8.
    pub const HASH_ALG: usize = 32;
    /// Header flags, u8.
    pub const HEADER_FLAGS: usize = 33;
    /// Reserved, 6 bytes, zero.
    pub const RESERVED0: usize = 34;
    /// Content hash, 32 bytes.
    pub const CONTENT_HASH: usize = 40;
    /// Engine build id, 16 bytes.
    pub const ENGINE_BUILD_ID: usize = 72;
    /// Bytecode ABI version (H2), u32.
    pub const BYTECODE_ABI: usize = 88;
    /// Value-encoding version (H6), u32.
    pub const VALUE_ENCODING: usize = 92;
    /// Object-layout version (H8, H17), u32.
    pub const OBJECT_LAYOUT: usize = 96;
    /// Barrier version (H11), u32.
    pub const BARRIER: usize = 100;
    /// Frame version (H14), u32.
    pub const FRAME: usize = 104;
    /// IC and feedback layout version (H16), u32.
    pub const IC: usize = 108;
    /// Deopt-metadata version (H29), u32.
    pub const DEOPT_META: usize = 112;
    /// Fact-schema version (H16, H37), u32.
    pub const FACT_SCHEMA: usize = 116;
    /// Target architecture code, u8.
    pub const TARGET_ARCH: usize = 120;
    /// Target ABI code, u8.
    pub const TARGET_ABI: usize = 121;
    /// log2 of the heap page size (H9), u8.
    pub const HEAP_PAGE_LOG2: usize = 122;
    /// Reserved, 1 byte, zero.
    pub const RESERVED1: usize = 123;
    /// Source flags, u32.
    pub const SOURCE_FLAGS: usize = 124;
    /// Required CPU features, u64 bitset.
    pub const CPU_REQUIRED: usize = 128;
    /// Builtin ABI version (H20), u32.
    pub const BUILTIN_ABI: usize = 136;
    /// Compat flags (container v2), u32: a bit this engine does not know is ignored.
    pub const COMPAT_FLAGS: usize = 140;
    /// Incompat flags (container v2), u32: a bit this engine does not know gives BYTECODE.
    pub const INCOMPAT_FLAGS: usize = 144;
    /// VM layout version (`ep9_vm::versions::VM_LAYOUT_VERSION`, H35-Q10), u32.
    pub const VM_LAYOUT: usize = 148;
    /// RuntimeTable length the native code needs (C-18), u32: at most the engine's.
    pub const RUNTIME_TABLE_LEN: usize = 152;
    /// The unit-data slot of the native code (H35-Q9, C-20), u8: 0-63.
    pub const UNIT_DATA_SLOT: usize = 156;
    /// Reserved, 3 bytes, zero.
    pub const RESERVED3: usize = 157;
    /// Hash of the snapshot root shapes the native code bakes as immediates (H35, H30), 8 bytes.
    pub const ROOT_SHAPES: usize = 160;
    /// Hash of the run-time flags that change emitted code, 8 bytes.
    pub const CODEGEN_FLAGS: usize = 168;
    /// Reserved up to the end of the header, zero.
    pub const RESERVED2: usize = 176;
}

/// Byte offsets inside one section table entry.
pub mod entry_offsets {
    /// Section kind, u32.
    pub const KIND: usize = 0;
    /// Section flags, u32: [`super::entry_flags`].
    pub const FLAGS: usize = 4;
    /// Offset from the start of the unit, u64.
    pub const OFFSET: usize = 8;
    /// Size in bytes, u64.
    pub const SIZE: usize = 16;
    /// log2 of the alignment, u8.
    pub const ALIGN_LOG2: usize = 24;
    /// Reserved, 7 bytes, zero.
    pub const RESERVED: usize = 25;
}

/// Section entry flag bits (entry offset 4; container v2).
pub mod entry_flags {
    /// Bit 0: the native code needs this section. Only an extension section (kind >= 0x100)
    /// may set it: an extension this engine does not know gives BYTECODE with the bit and is
    /// skipped without it. A named kind has flags 0.
    pub const NATIVE_NEEDS: u32 = 1;
    /// Every defined bit.
    pub const DEFINED: u32 = NATIVE_NEEDS;
}

/// Header compat flag bits (offset 140; container v2). A bit this engine does not know is
/// ignored: a writer sets one for an optional feature that an older engine may skip.
pub mod compat_flags {
    /// Bit 0 (container v3, MC-w23-15): the text has landing pads. Every indirect entry of the
    /// text (each function entry) starts with a landing pad (AArch64 `bti c`) and every
    /// indirect jump goes through x16 or x17, so a loader may map the text with BTI
    /// (`PROT_BTI`). Without the bit a loader maps the text without BTI. An engine that does not
    /// know the bit maps without BTI, which is safe.
    pub const HAS_PADS: u32 = 1;
    /// Every defined bit.
    pub const DEFINED: u32 = HAS_PADS;
}

/// Header incompat flag bits (offset 144; container v2). A bit this engine does not know gives
/// BYTECODE: the native code needs a feature this engine does not have. None is defined yet.
pub mod incompat_flags {
    /// Every defined bit.
    pub const DEFINED: u32 = 0;
}

/// The first extension section kind (container v2). An extension is covered by the content
/// hash; its entry flags say if the native code needs it ([`entry_flags::NATIVE_NEEDS`]).
pub const EXTENSION_KIND_MIN: u32 = 0x100;
/// The alignment range of an extension section (log2): 8 B to 64 KiB.
pub const EXTENSION_ALIGN_LOG2: core::ops::RangeInclusive<u8> = 3..=16;
/// The number of unit-data slots (H35-Q9): `Cx::unit_data` has 64 words. Slot 0 is the
/// engine's own unit (C-20).
pub const UNIT_DATA_SLOTS: u8 = 64;

/// Why a unit fails S4 step 1 or 2 (a LOAD ERROR).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadUnit {
    /// Shorter than the frozen prefix or the header.
    Truncated,
    /// Wrong magic.
    Magic,
    /// Another container version.
    ContainerVersion(u32),
    /// Header size is not [`HEADER_SIZE`].
    HeaderSize,
    /// Unknown hash algorithm.
    HashAlg,
    /// An undefined header flag bit.
    HeaderFlags,
    /// A reserved header byte is not zero.
    Reserved,
    /// Invalid source flags.
    SourceFlags,
    /// `total_size` is not the input length.
    TotalSize,
    /// The section table is out of bounds or not canonical.
    SectionTable(&'static str),
    /// A required section is missing.
    MissingSection(SectionKind),
    /// The signed flag and the signature section disagree.
    Signature,
    /// The content hash does not match.
    Hash,
}

impl fmt::Display for BadUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("truncated header"),
            Self::Magic => f.write_str("not an ep9 unit (bad magic)"),
            Self::ContainerVersion(v) => {
                write!(f, "container version {v}, expected {CONTAINER_VERSION}")
            }
            Self::HeaderSize => f.write_str("bad header size"),
            Self::HashAlg => f.write_str("unknown hash algorithm"),
            Self::HeaderFlags => f.write_str("undefined header flag"),
            Self::Reserved => f.write_str("reserved header bytes are not zero"),
            Self::SourceFlags => f.write_str("invalid source flags"),
            Self::TotalSize => f.write_str("total size does not match the input length"),
            Self::SectionTable(what) => write!(f, "bad section table: {what}"),
            Self::MissingSection(k) => write!(f, "missing {} section", k.name()),
            Self::Signature => f.write_str("signed flag and signature section disagree"),
            Self::Hash => f.write_str("content hash mismatch"),
        }
    }
}

impl std::error::Error for BadUnit {}

/// The header fields that vary between units. Magic, container version, header size, section
/// table offset and hash algorithm have one value in container version 2: [`UnitHeader::encode`]
/// writes it and [`UnitHeader::decode`] requires it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitHeader {
    /// Size of the whole unit in bytes.
    pub total_size: u64,
    /// Number of section table entries.
    pub section_count: u32,
    /// [`header_flags`] bits.
    pub header_flags: u8,
    /// SHA-256 of the unit with this field and the signature section set to zero.
    pub content_hash: [u8; 32],
    /// The engine build the unit was compiled for (H35-Q8). Covered by the hash.
    pub engine_build_id: [u8; 16],
    /// Contract versions.
    pub versions: Versions,
    /// [`crate::target_arch`] code.
    pub target_arch: u8,
    /// [`crate::target_abi`] code.
    pub target_abi: u8,
    /// log2 of the heap page size the native code was built for (H9-Q4, H35-Q6).
    pub heap_page_log2: u8,
    /// Goal, strict and TS flags for the SOURCE path (CompiledUnit flag bits).
    pub source_flags: u32,
    /// [`crate::cpu`] bits the native code needs.
    pub cpu_required: u64,
    /// [`compat_flags`] bits: an unknown bit is ignored.
    pub compat_flags: u32,
    /// [`incompat_flags`] bits: an unknown bit gives BYTECODE.
    pub incompat_flags: u32,
    /// The RuntimeTable entries the native code calls: entries `0..runtime_table_len`.
    pub runtime_table_len: u32,
    /// The unit-data slot the native code reads its RW copy through (H35-Q9; C-20).
    pub unit_data_slot: u8,
    /// Hash of the snapshot root shapes the native code bakes as immediates; 0 = none.
    pub root_shapes: [u8; 8],
    /// Hash of the run-time flags that changed the emitted code; 0 = none.
    pub codegen_flags: [u8; 8],
}

fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put_u64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

pub(crate) fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

pub(crate) fn le_u64(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

impl UnitHeader {
    /// The 256 header bytes. Reserved bytes are zero.
    pub fn encode(&self) -> [u8; HEADER_SIZE] {
        let mut b = [0u8; HEADER_SIZE];
        b[offsets::MAGIC..offsets::MAGIC + 8].copy_from_slice(&MAGIC);
        put_u32(&mut b, offsets::CONTAINER_VERSION, CONTAINER_VERSION);
        put_u32(&mut b, offsets::HEADER_SIZE, HEADER_SIZE as u32);
        put_u64(&mut b, offsets::TOTAL_SIZE, self.total_size);
        put_u32(&mut b, offsets::SECTION_COUNT, self.section_count);
        put_u32(
            &mut b,
            offsets::SECTION_TABLE_OFFSET,
            SECTION_TABLE_OFFSET as u32,
        );
        b[offsets::HASH_ALG] = HASH_SHA256;
        b[offsets::HEADER_FLAGS] = self.header_flags;
        b[offsets::CONTENT_HASH..offsets::CONTENT_HASH + 32].copy_from_slice(&self.content_hash);
        b[offsets::ENGINE_BUILD_ID..offsets::ENGINE_BUILD_ID + 16]
            .copy_from_slice(&self.engine_build_id);
        for c in Contract::ALL {
            put_u32(&mut b, c.offset(), self.versions.get(c));
        }
        b[offsets::TARGET_ARCH] = self.target_arch;
        b[offsets::TARGET_ABI] = self.target_abi;
        b[offsets::HEAP_PAGE_LOG2] = self.heap_page_log2;
        put_u32(&mut b, offsets::SOURCE_FLAGS, self.source_flags);
        put_u64(&mut b, offsets::CPU_REQUIRED, self.cpu_required);
        put_u32(&mut b, offsets::COMPAT_FLAGS, self.compat_flags);
        put_u32(&mut b, offsets::INCOMPAT_FLAGS, self.incompat_flags);
        put_u32(&mut b, offsets::RUNTIME_TABLE_LEN, self.runtime_table_len);
        b[offsets::UNIT_DATA_SLOT] = self.unit_data_slot;
        b[offsets::ROOT_SHAPES..offsets::ROOT_SHAPES + 8].copy_from_slice(&self.root_shapes);
        b[offsets::CODEGEN_FLAGS..offsets::CODEGEN_FLAGS + 8].copy_from_slice(&self.codegen_flags);
        b
    }

    /// Reads the header at the start of `bytes` (S4 step 1, the header part). It checks every
    /// rule the header alone decides: prefix, hash algorithm, flags, reserved bytes, source
    /// flags and the section count range. It does not check the total size, the section table
    /// or the hash; [`crate::check()`] does.
    pub fn decode(bytes: &[u8]) -> Result<UnitHeader, BadUnit> {
        if bytes.len() < FROZEN_PREFIX_SIZE {
            return Err(BadUnit::Truncated);
        }
        if bytes[offsets::MAGIC..offsets::MAGIC + 8] != MAGIC {
            return Err(BadUnit::Magic);
        }
        let container_version = le_u32(bytes, offsets::CONTAINER_VERSION);
        if container_version != CONTAINER_VERSION {
            return Err(BadUnit::ContainerVersion(container_version));
        }
        if le_u32(bytes, offsets::HEADER_SIZE) as usize != HEADER_SIZE {
            return Err(BadUnit::HeaderSize);
        }
        if le_u32(bytes, offsets::SECTION_TABLE_OFFSET) as usize != SECTION_TABLE_OFFSET {
            return Err(BadUnit::SectionTable("table offset"));
        }
        let section_count = le_u32(bytes, offsets::SECTION_COUNT);
        if !(MIN_SECTIONS..=MAX_SECTIONS).contains(&section_count) {
            return Err(BadUnit::SectionTable("section count"));
        }
        if bytes.len() < HEADER_SIZE {
            return Err(BadUnit::Truncated);
        }
        if bytes[offsets::HASH_ALG] != HASH_SHA256 {
            return Err(BadUnit::HashAlg);
        }
        let header_flags = bytes[offsets::HEADER_FLAGS];
        if header_flags & !header_flags::DEFINED != 0 {
            return Err(BadUnit::HeaderFlags);
        }
        let reserved = [
            offsets::RESERVED0..offsets::CONTENT_HASH,
            offsets::RESERVED1..offsets::SOURCE_FLAGS,
            offsets::RESERVED3..offsets::ROOT_SHAPES,
            offsets::RESERVED2..HEADER_SIZE,
        ];
        if reserved
            .into_iter()
            .any(|r| bytes[r].iter().any(|&x| x != 0))
        {
            return Err(BadUnit::Reserved);
        }
        let source_flags = le_u32(bytes, offsets::SOURCE_FLAGS);
        if !source_flags_valid(source_flags) {
            return Err(BadUnit::SourceFlags);
        }
        let mut versions = Versions::default();
        for c in Contract::ALL {
            versions.set(c, le_u32(bytes, c.offset()));
        }
        let eight = |at: usize| -> [u8; 8] { bytes[at..at + 8].try_into().expect("8 bytes") };
        let mut h = UnitHeader {
            total_size: le_u64(bytes, offsets::TOTAL_SIZE),
            section_count,
            header_flags,
            content_hash: [0; 32],
            engine_build_id: [0; 16],
            versions,
            target_arch: bytes[offsets::TARGET_ARCH],
            target_abi: bytes[offsets::TARGET_ABI],
            heap_page_log2: bytes[offsets::HEAP_PAGE_LOG2],
            source_flags,
            cpu_required: le_u64(bytes, offsets::CPU_REQUIRED),
            compat_flags: le_u32(bytes, offsets::COMPAT_FLAGS),
            incompat_flags: le_u32(bytes, offsets::INCOMPAT_FLAGS),
            runtime_table_len: le_u32(bytes, offsets::RUNTIME_TABLE_LEN),
            unit_data_slot: bytes[offsets::UNIT_DATA_SLOT],
            root_shapes: eight(offsets::ROOT_SHAPES),
            codegen_flags: eight(offsets::CODEGEN_FLAGS),
        };
        h.content_hash
            .copy_from_slice(&bytes[offsets::CONTENT_HASH..offsets::CONTENT_HASH + 32]);
        h.engine_build_id
            .copy_from_slice(&bytes[offsets::ENGINE_BUILD_ID..offsets::ENGINE_BUILD_ID + 16]);
        Ok(h)
    }
}

/// One section table entry of a named kind. Flags (0 for a named kind) and alignment (fixed
/// per kind) are not stored: [`SectionEntry::encode`] writes 0 and the kind's alignment, and
/// [`SectionEntry::decode`] requires them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionEntry {
    /// The kind.
    pub kind: SectionKind,
    /// Offset from the start of the unit.
    pub offset: u64,
    /// Size in bytes.
    pub size: u64,
}

impl SectionEntry {
    /// The 32 entry bytes.
    pub fn encode(&self) -> [u8; SECTION_ENTRY_SIZE] {
        let mut b = [0u8; SECTION_ENTRY_SIZE];
        put_u32(&mut b, entry_offsets::KIND, self.kind as u32);
        put_u64(&mut b, entry_offsets::OFFSET, self.offset);
        put_u64(&mut b, entry_offsets::SIZE, self.size);
        b[entry_offsets::ALIGN_LOG2] = self.kind.align_log2();
        b
    }

    /// Reads one entry of a named kind. `b` must be at least [`SECTION_ENTRY_SIZE`] bytes;
    /// extra bytes are ignored. Bounds and order are the caller's check. An extension entry is
    /// an error here: [`TableEntry::decode`] reads both.
    pub fn decode(b: &[u8]) -> Result<SectionEntry, BadUnit> {
        match TableEntry::decode(b)? {
            TableEntry::Named(e) => Ok(e),
            TableEntry::Extension(_) => Err(BadUnit::SectionTable("unknown section kind")),
        }
    }
}

/// One extension section entry (kind >= [`EXTENSION_KIND_MIN`]; container v2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtensionEntry {
    /// The kind, at least [`EXTENSION_KIND_MIN`].
    pub kind: u32,
    /// [`entry_flags::NATIVE_NEEDS`]: the native code needs this section.
    pub native_needs: bool,
    /// log2 of the alignment, in [`EXTENSION_ALIGN_LOG2`].
    pub align_log2: u8,
    /// Offset from the start of the unit.
    pub offset: u64,
    /// Size in bytes.
    pub size: u64,
}

impl ExtensionEntry {
    /// The 32 entry bytes.
    pub fn encode(&self) -> [u8; SECTION_ENTRY_SIZE] {
        let mut b = [0u8; SECTION_ENTRY_SIZE];
        put_u32(&mut b, entry_offsets::KIND, self.kind);
        let flags = if self.native_needs {
            entry_flags::NATIVE_NEEDS
        } else {
            0
        };
        put_u32(&mut b, entry_offsets::FLAGS, flags);
        put_u64(&mut b, entry_offsets::OFFSET, self.offset);
        put_u64(&mut b, entry_offsets::SIZE, self.size);
        b[entry_offsets::ALIGN_LOG2] = self.align_log2;
        b
    }
}

/// One section table entry: a named kind or an extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableEntry {
    /// Kinds 1-11 ([`SectionKind`]).
    Named(SectionEntry),
    /// Kinds >= [`EXTENSION_KIND_MIN`].
    Extension(ExtensionEntry),
}

impl TableEntry {
    /// The kind id.
    pub fn kind(&self) -> u32 {
        match self {
            Self::Named(e) => e.kind as u32,
            Self::Extension(e) => e.kind,
        }
    }

    /// Offset from the start of the unit.
    pub fn offset(&self) -> u64 {
        match self {
            Self::Named(e) => e.offset,
            Self::Extension(e) => e.offset,
        }
    }

    /// Size in bytes.
    pub fn size(&self) -> u64 {
        match self {
            Self::Named(e) => e.size,
            Self::Extension(e) => e.size,
        }
    }

    /// log2 of the alignment.
    pub fn align_log2(&self) -> u8 {
        match self {
            Self::Named(e) => e.kind.align_log2(),
            Self::Extension(e) => e.align_log2,
        }
    }

    /// The 32 entry bytes.
    pub fn encode(&self) -> [u8; SECTION_ENTRY_SIZE] {
        match self {
            Self::Named(e) => e.encode(),
            Self::Extension(e) => e.encode(),
        }
    }

    /// Reads one entry. `b` must be at least [`SECTION_ENTRY_SIZE`] bytes; extra bytes are
    /// ignored. Bounds and order are the caller's check. A kind below the extension range that
    /// is not named, an undefined flag bit, a flag on a named kind, a wrong alignment or a
    /// nonzero reserved byte is a LOAD ERROR.
    pub fn decode(b: &[u8]) -> Result<TableEntry, BadUnit> {
        if b.len() < SECTION_ENTRY_SIZE {
            return Err(BadUnit::SectionTable("truncated table"));
        }
        let raw = le_u32(b, entry_offsets::KIND);
        let flags = le_u32(b, entry_offsets::FLAGS);
        if flags & !entry_flags::DEFINED != 0 {
            return Err(BadUnit::SectionTable("section flags"));
        }
        if b[entry_offsets::RESERVED..SECTION_ENTRY_SIZE]
            .iter()
            .any(|&x| x != 0)
        {
            return Err(BadUnit::SectionTable("reserved entry bytes"));
        }
        let (offset, size) = (
            le_u64(b, entry_offsets::OFFSET),
            le_u64(b, entry_offsets::SIZE),
        );
        let align_log2 = b[entry_offsets::ALIGN_LOG2];
        if raw >= EXTENSION_KIND_MIN {
            if !EXTENSION_ALIGN_LOG2.contains(&align_log2) {
                return Err(BadUnit::SectionTable("section alignment"));
            }
            return Ok(TableEntry::Extension(ExtensionEntry {
                kind: raw,
                native_needs: flags & entry_flags::NATIVE_NEEDS != 0,
                align_log2,
                offset,
                size,
            }));
        }
        let kind =
            SectionKind::from_u32(raw).ok_or(BadUnit::SectionTable("unknown section kind"))?;
        if flags != 0 {
            return Err(BadUnit::SectionTable("section flags"));
        }
        if align_log2 != kind.align_log2() {
            return Err(BadUnit::SectionTable("section alignment"));
        }
        Ok(TableEntry::Named(SectionEntry { kind, offset, size }))
    }
}
