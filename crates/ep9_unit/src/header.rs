//! Unit header v0 (H35-Q1): 256 bytes, little-endian, every field at a fixed offset, 8-byte
//! aligned. A section table follows it, then the sections.
//!
//! ```text
//!   0 magic "\x89EP9UN\r\n"      8 container_version u32 (1)    12 header_size u32 (256)
//!  16 total_size u64            24 section_count u32           28 section_table_offset u32 (256)
//!     -- 32-byte frozen prefix: the same place and meaning in every container version --
//!  32 hash_alg u8 (1 = SHA-256)  33 header_flags u8 (bit 0 signed)  34 reserved [6] = 0
//!  40 content_hash [32]         72 engine_build_id [16]
//!  88 bytecode_abi  92 value_encoding  96 object_layout  100 barrier  104 frame  108 ic
//! 112 deopt_meta   116 fact_schema                                          (u32 each)
//! 120 target_arch u8  121 target_abi u8  122 heap_page_log2 u8  123 reserved u8 = 0
//! 124 source_flags u32 (ep9_bytecode unit flag bits)  128 cpu_required u64
//! 136 builtin_abi u32  140..256 reserved = 0
//!
//! section entry (32 B, table at offset 256, one per kind, ascending kind):
//!   0 kind u32  4 flags u32 = 0  8 offset u64  16 size u64  24 align_log2 u8  25 reserved [7] = 0
//! ```
//!
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
pub const CONTAINER_VERSION: u32 = 1;
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
/// The most sections a unit has: one per kind.
pub const MAX_SECTIONS: u32 = SectionKind::ALL.len() as u32;

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
    /// Reserved up to the end of the header, zero.
    pub const RESERVED2: usize = 140;
}

/// Byte offsets inside one section table entry.
pub mod entry_offsets {
    /// Section kind, u32.
    pub const KIND: usize = 0;
    /// Section flags, u32; none defined, zero.
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
/// table offset and hash algorithm have one value in container version 1: [`UnitHeader::encode`]
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
        };
        h.content_hash
            .copy_from_slice(&bytes[offsets::CONTENT_HASH..offsets::CONTENT_HASH + 32]);
        h.engine_build_id
            .copy_from_slice(&bytes[offsets::ENGINE_BUILD_ID..offsets::ENGINE_BUILD_ID + 16]);
        Ok(h)
    }
}

/// One section table entry. Flags (none defined) and alignment (fixed per kind) are not
/// stored: [`SectionEntry::encode`] writes 0 and the kind's alignment, and
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

    /// Reads one entry. `b` must be at least [`SECTION_ENTRY_SIZE`] bytes; extra bytes are
    /// ignored. Bounds and order are the caller's check.
    pub fn decode(b: &[u8]) -> Result<SectionEntry, BadUnit> {
        if b.len() < SECTION_ENTRY_SIZE {
            return Err(BadUnit::SectionTable("truncated table"));
        }
        let kind = SectionKind::from_u32(le_u32(b, entry_offsets::KIND))
            .ok_or(BadUnit::SectionTable("unknown section kind"))?;
        if le_u32(b, entry_offsets::FLAGS) != 0 {
            return Err(BadUnit::SectionTable("section flags"));
        }
        if b[entry_offsets::ALIGN_LOG2] != kind.align_log2() {
            return Err(BadUnit::SectionTable("section alignment"));
        }
        if b[entry_offsets::RESERVED..SECTION_ENTRY_SIZE]
            .iter()
            .any(|&x| x != 0)
        {
            return Err(BadUnit::SectionTable("reserved entry bytes"));
        }
        Ok(SectionEntry {
            kind,
            offset: le_u64(b, entry_offsets::OFFSET),
            size: le_u64(b, entry_offsets::SIZE),
        })
    }
}
