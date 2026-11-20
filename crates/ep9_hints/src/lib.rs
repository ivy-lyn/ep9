//! The `.ep9h` TS hint side table, schema v0 (H37, `docs/adr/H37-hint-side-table.md`;
//! `docs/plan/05-aot.md` section 7). `ep9-typecheck` writes it from tsrs; the join in
//! `ep9_frontend` reads it. Nothing in the engine reads it before A4 (H37 S9).
//!
//! ```text
//!  .ep9h, all integers little-endian, no padding, the parts in this order:
//!  +- header      64 B: magic | schema version | fact-schema version | TS flags |
//!  |                    tsrs commit | the count of each part | reserved
//!  +- file table  52 B each, sorted by path: path | record range | decoded-text hash |
//!  |                    has_type_errors
//!  +- classes     20 B each, class id = index: decl file:offset | slot count | field range
//!  +- fields      8 B each: the decl file:offset of each field, in slot order
//!  +- records     32 B each, per file, sorted by (start, end, kind): repr category |
//!  |                    class id | slot candidate | call-target decl | non-null | enum const |
//!  |                    element type | dts/lib | confidence
//!  +- strings     the paths of the file table, end to end; nothing else (no type strings)
//!
//!  write(&Hints) ──> bytes ──> read(bytes, &Expect) ──> Ok(Hints) | Err(DropReason)
//!  Hints::file_hints(path, text_hash) ──> Ok(&[Record]) | Err(DropReason)
//! ```
//!
//! Rules:
//! - Zero dependencies (H37 Rules). The caller computes the SHA-256 of the decoded text
//!   (H37-Q3); this crate only stores and compares the 32 bytes.
//! - Offsets are UTF-8 byte offsets of the decoded text after the BOM strip (H37-Q4, S2).
//! - [`read()`] never panics. A bad, short or mismatched input gives one [`DropReason`] and no
//!   hints; [`DropCounts`] counts them. A header mismatch (schema version, fact-schema version,
//!   TS flags) ignores the whole input (S5).
//! - The encoding is canonical: [`read()`] accepts exactly the bytes that [`write()`] makes, so
//!   `write(read(b)) == b` for every accepted `b`.
//! - Each code list ([`NodeKind`], [`Repr`]) is append-only: a code never changes its meaning.
//!   An append or any layout change bumps [`HINT_SCHEMA_VERSION`], so an old reader ignores a
//!   new file (S5) and does not drop it as [`DropReason::BadRecord`].
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod codes;
mod model;
mod reader;
mod writer;

pub use codes::{NodeKind, Repr};
pub use model::{
    ClassEntry, DeclRef, FileEntry, Header, Hints, Record, TextHash, TsFlags, TsrsCommit,
    commit_hex, parse_commit_hex,
};
pub use reader::{DropCounts, DropReason, Expect, read};
pub use writer::{WriteError, write};

/// The `.ep9h` schema version. Version 1 is schema v0 of H37 (as `FACT_SCHEMA_VERSION` 1 is
/// FactSnapshot v0), so a zeroed header never matches.
///
/// | Version | Change |
/// |---|---|
/// | 1 | Schema v0 (H37; M10-P1 A1H-SCHEMA) |
pub const HINT_SCHEMA_VERSION: u32 = 1;

/// The first 8 bytes of a `.ep9h` file: `"\x89EP9HT\r\n"` (PNG style, like the unit and
/// FactSnapshot magics).
pub const MAGIC: [u8; 8] = *b"\x89EP9HT\r\n";

/// The size of the header in bytes.
pub const HEADER_SIZE: usize = 64;
/// The size of one file-table entry in bytes.
pub const FILE_ENTRY_SIZE: usize = 52;
/// The size of one class entry in bytes.
pub const CLASS_ENTRY_SIZE: usize = 20;
/// The size of one field entry in bytes.
pub const FIELD_SIZE: usize = 8;
/// The size of one record in bytes (H37-Q1: fixed size).
pub const RECORD_SIZE: usize = 32;

/// The null value of a `u32` id field: class id and call-target file.
pub const NULL_U32: u32 = u32::MAX;
/// The null value of the `u16` slot-candidate field.
pub const NULL_U16: u16 = u16::MAX;

/// Byte offsets of the fields in each fixed-size part. Tests use them to make bad inputs.
pub mod offsets {
    /// Header: the magic, 8 bytes.
    pub const MAGIC: usize = 0;
    /// Header: the schema version, `u32`.
    pub const SCHEMA_VERSION: usize = 8;
    /// Header: the fact-schema version (`ep9_facts::FACT_SCHEMA_VERSION`), `u32`.
    pub const FACT_SCHEMA_VERSION: usize = 12;
    /// Header: the TS flags, `u32` ([`crate::TsFlags`]; bits 3-31 are 0).
    pub const TS_FLAGS: usize = 16;
    /// Header: the tsrs commit, 20 bytes (the SHA-1 of `third_party/tsrs.commit`).
    pub const TSRS_COMMIT: usize = 20;
    /// Header: the number of file-table entries, `u32`.
    pub const FILE_COUNT: usize = 40;
    /// Header: the number of class entries, `u32`.
    pub const CLASS_COUNT: usize = 44;
    /// Header: the number of field entries, `u32`.
    pub const FIELD_COUNT: usize = 48;
    /// Header: the number of records, `u32`.
    pub const RECORD_COUNT: usize = 52;
    /// Header: the size of the strings part in bytes, `u32`.
    pub const STRING_BYTES: usize = 56;
    /// Header: reserved, `u32`, 0.
    pub const HEADER_RESERVED: usize = 60;

    /// File entry: the path offset in the strings part, `u32`.
    pub const FILE_PATH_OFFSET: usize = 0;
    /// File entry: the path length in bytes, `u32`.
    pub const FILE_PATH_LEN: usize = 4;
    /// File entry: the index of the file's first record, `u32`.
    pub const FILE_FIRST_RECORD: usize = 8;
    /// File entry: the number of the file's records, `u32`.
    pub const FILE_RECORD_COUNT: usize = 12;
    /// File entry: the decoded-text hash, 32 bytes.
    pub const FILE_TEXT_HASH: usize = 16;
    /// File entry: `has_type_errors`, `u8`, 0 or 1.
    pub const FILE_HAS_TYPE_ERRORS: usize = 48;
    /// File entry: reserved, 3 bytes, 0.
    pub const FILE_RESERVED: usize = 49;

    /// Class entry: the decl file index, `u32`.
    pub const CLASS_DECL_FILE: usize = 0;
    /// Class entry: the decl offset, `u32`.
    pub const CLASS_DECL_OFFSET: usize = 4;
    /// Class entry: the slot count, `u32`.
    pub const CLASS_SLOT_COUNT: usize = 8;
    /// Class entry: the index of the class's first field, `u32`.
    pub const CLASS_FIRST_FIELD: usize = 12;
    /// Class entry: the number of the class's fields, `u32`.
    pub const CLASS_FIELD_COUNT: usize = 16;

    /// Field entry: the decl file index, `u32` (an inherited field names its base class file).
    pub const FIELD_FILE: usize = 0;
    /// Field entry: the decl offset, `u32`.
    pub const FIELD_OFFSET: usize = 4;

    /// Record: the key start, `u32`.
    pub const REC_START: usize = 0;
    /// Record: the key end, `u32`.
    pub const REC_END: usize = 4;
    /// Record: the key node kind, `u16` ([`crate::NodeKind`]).
    pub const REC_KIND: usize = 8;
    /// Record: the repr category, `u8` ([`crate::Repr`]).
    pub const REC_REPR: usize = 10;
    /// Record: the element type, `u8` ([`crate::Repr`]).
    pub const REC_ELEMENT: usize = 11;
    /// Record: the flags, `u8`: bit 0 non-null, bit 1 dts/lib, bit 2 enum const; bits 3-7 are 0.
    pub const REC_FLAGS: usize = 12;
    /// Record: the confidence, `u8` (0 none given, 1 lowest to 255 highest; H30-Q1).
    pub const REC_CONFIDENCE: usize = 13;
    /// Record: the slot candidate, `u16` ([`crate::NULL_U16`] = none).
    pub const REC_SLOT: usize = 14;
    /// Record: the class id, `u32` ([`crate::NULL_U32`] = none).
    pub const REC_CLASS: usize = 16;
    /// Record: the call-target decl file, `u32` ([`crate::NULL_U32`] = none).
    pub const REC_TARGET_FILE: usize = 20;
    /// Record: the call-target decl offset, `u32` (0 when the file is none).
    pub const REC_TARGET_OFFSET: usize = 24;
    /// Record: the enum const, `i32` (0 when flag bit 2 is clear).
    pub const REC_ENUM_CONST: usize = 28;
}
