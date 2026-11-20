//! The decoded form of a `.ep9h` file.

use crate::codes::{NodeKind, Repr};
use crate::reader::DropReason;

/// The SHA-256 of a file's decoded UTF-8 text after the BOM strip (H37-Q3; the H02 F12
/// source hash). The caller computes it.
pub type TextHash = [u8; 32];

/// The tsrs commit (a SHA-1, 20 bytes) that the writer ran. The reader does not check it;
/// it is for reports.
pub type TsrsCommit = [u8; 20];

/// The three TS flags of H3, as in the unit header. The reader ignores a file whose flags
/// differ from the caller's flags (S5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TsFlags {
    /// `useDefineForClassFields` (bit 0).
    pub use_define_for_class_fields: bool,
    /// `experimentalDecorators` (bit 1).
    pub experimental_decorators: bool,
    /// `emitDecoratorMetadata` (bit 2).
    pub emit_decorator_metadata: bool,
}

impl TsFlags {
    /// The `u32` of the header.
    pub const fn to_bits(self) -> u32 {
        (self.use_define_for_class_fields as u32)
            | ((self.experimental_decorators as u32) << 1)
            | ((self.emit_decorator_metadata as u32) << 2)
    }

    /// The flags of a header `u32`, or `None` when a bit above bit 2 is set.
    pub const fn from_bits(bits: u32) -> Option<TsFlags> {
        if bits & !0b111 != 0 {
            return None;
        }
        Some(TsFlags {
            use_define_for_class_fields: bits & 1 != 0,
            experimental_decorators: bits & 2 != 0,
            emit_decorator_metadata: bits & 4 != 0,
        })
    }
}

/// The header fields that are not counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    /// The tsrs commit of the writer.
    pub tsrs_commit: TsrsCommit,
    /// The `ep9_facts::FACT_SCHEMA_VERSION` of the writer.
    pub fact_schema_version: u32,
    /// The TS flags that tsrs checked with.
    pub ts_flags: TsFlags,
}

/// A position in a source file of the file table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DeclRef {
    /// The index of the file in the file table.
    pub file: u32,
    /// The UTF-8 byte offset in that file (H37-Q4).
    pub offset: u32,
}

/// One entry of the file table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    /// The path: not empty, no NUL. The table is sorted by path (byte order), each path once.
    pub path: String,
    /// The decoded-text hash (H37-Q3).
    pub text_hash: TextHash,
    /// tsrs found a type error in this file: its hints are dropped (S3).
    pub has_type_errors: bool,
    /// The index of the file's first record. Each file's records follow the previous file's.
    pub first_record: u32,
    /// The number of the file's records.
    pub record_count: u32,
}

/// One entry of the class table. The class id is the index of the entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassEntry {
    /// Where the class is declared.
    pub decl: DeclRef,
    /// The slot count of an instance.
    pub slot_count: u32,
    /// The index of the class's first field in [`Hints::fields`]. Each class's fields follow
    /// the previous class's.
    pub first_field: u32,
    /// The number of the class's fields: field `i` has slot `i`, so at most `slot_count`.
    pub field_count: u32,
}

/// One hint record. Its key is (start, end, kind) in the file that owns it (S1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    /// The key start: a UTF-8 byte offset (H37-Q4).
    pub start: u32,
    /// The key end: a UTF-8 byte offset, at least `start`.
    pub end: u32,
    /// The key node kind (H37-Q2).
    pub kind: NodeKind,
    /// The repr category ([`Repr::None`] = no hint).
    pub repr: Repr,
    /// The element type of an array ([`Repr::None`] = no hint).
    pub element: Repr,
    /// The value is never `null` or `undefined` (false = no claim).
    pub non_null: bool,
    /// The type comes from a `.d.ts` file or a lib file.
    pub dts_lib: bool,
    /// The constant value of an enum member read.
    pub enum_const: Option<i32>,
    /// 0 = none given, 1 lowest to 255 highest (as the H30-Q1 confidence byte).
    pub confidence: u8,
    /// The class id, an index of [`Hints::classes`]. On a member-access kind (3
    /// StaticMemberExpression, 4 ComputedMemberExpression, 5 PrivateFieldExpression) it is the
    /// receiver's class, which owns `slot` (05-aot.md:149 "class receiver -> shape + slot
    /// candidate"). On any other kind it is the class of the node's value, and `repr` must be
    /// [`Repr::ClassInstance`] (else the reader gives [`DropReason::BadRecord`]).
    pub class: Option<u32>,
    /// The slot of the member in the receiver class: below its slot count. Needs `class` and
    /// a member-access kind (3-5).
    pub slot: Option<u16>,
    /// The declaration of the call target.
    pub call_target: Option<DeclRef>,
}

impl Record {
    /// The sort key of the records of one file.
    pub const fn key(&self) -> (u32, u32, u16) {
        (self.start, self.end, self.kind.code())
    }
}

/// A decoded `.ep9h` file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hints {
    /// The header.
    pub header: Header,
    /// The file table.
    pub files: Vec<FileEntry>,
    /// The class table.
    pub classes: Vec<ClassEntry>,
    /// The field decls of every class, class after class (an inherited field names its base
    /// class file).
    pub fields: Vec<DeclRef>,
    /// The records of every file, file after file.
    pub records: Vec<Record>,
}

impl Hints {
    /// The index of the file with this path.
    pub fn find_file(&self, path: &str) -> Option<usize> {
        self.files
            .binary_search_by(|f| f.path.as_bytes().cmp(path.as_bytes()))
            .ok()
    }

    /// The records of one file, for the join. `text_hash` is the caller's hash of the decoded
    /// text. A file with type errors or another hash gives a drop (S3), as does a path that is
    /// not in the table.
    pub fn file_hints(&self, path: &str, text_hash: &TextHash) -> Result<&[Record], DropReason> {
        let file = self
            .find_file(path)
            .and_then(|i| self.files.get(i))
            .ok_or(DropReason::NotInTable)?;
        if file.has_type_errors {
            return Err(DropReason::TypeErrors);
        }
        if &file.text_hash != text_hash {
            return Err(DropReason::HashMismatch);
        }
        let start = file.first_record as usize;
        let end = start.saturating_add(file.record_count as usize);
        self.records.get(start..end).ok_or(DropReason::BadFileTable)
    }

    /// The field decls of one class, in slot order.
    pub fn class_fields(&self, class: &ClassEntry) -> &[DeclRef] {
        let start = class.first_field as usize;
        let end = start.saturating_add(class.field_count as usize);
        self.fields.get(start..end).unwrap_or(&[])
    }
}

/// The 40 hex digits of a commit, as `third_party/tsrs.commit` holds them.
pub fn commit_hex(commit: &TsrsCommit) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(40);
    for b in commit {
        s.push(char::from(HEX[usize::from(b >> 4)]));
        s.push(char::from(HEX[usize::from(b & 15)]));
    }
    s
}

/// The commit of 40 hex digits (either case; spaces and a line end around them are allowed),
/// or `None`.
pub fn parse_commit_hex(text: &str) -> Option<TsrsCommit> {
    let digits = text.trim().as_bytes();
    if digits.len() != 40 {
        return None;
    }
    let mut out = [0u8; 20];
    for (o, pair) in out.iter_mut().zip(digits.as_chunks::<2>().0) {
        let hi = char::from(pair[0]).to_digit(16)?;
        let lo = char::from(pair[1]).to_digit(16)?;
        *o = u8::try_from(hi * 16 + lo).ok()?;
    }
    Some(out)
}
