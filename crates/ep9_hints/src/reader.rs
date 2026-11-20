//! The `.ep9h` reader. It never panics and never reads out of bounds: it checks the size of
//! every part against the input before it makes any vector, and every read is in a part that
//! it has checked.

use crate::codes::{NodeKind, Repr};
use crate::model::{ClassEntry, DeclRef, FileEntry, Header, Hints, Record, TsFlags};
use crate::offsets as o;
use crate::{
    CLASS_ENTRY_SIZE, FIELD_SIZE, FILE_ENTRY_SIZE, HEADER_SIZE, HINT_SCHEMA_VERSION, MAGIC,
    NULL_U16, NULL_U32, RECORD_SIZE,
};

/// The values that the caller's build must match (S5). The schema version is
/// [`HINT_SCHEMA_VERSION`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Expect {
    /// The caller's `ep9_facts::FACT_SCHEMA_VERSION`.
    pub fact_schema_version: u32,
    /// The caller's TS flags for the program.
    pub ts_flags: TsFlags,
}

/// Why hints were dropped. [`read()`] gives a reason that drops the whole input;
/// [`crate::Hints::file_hints`] gives a reason that drops one file's hints (S3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DropReason {
    /// The input ends before the header or a part that the header counts.
    Truncated,
    /// The input has bytes after the strings part.
    TrailingBytes,
    /// The first 8 bytes are not [`MAGIC`].
    BadMagic,
    /// The schema version is not [`HINT_SCHEMA_VERSION`] (S5).
    SchemaVersion,
    /// The fact-schema version is not the caller's (S5).
    FactSchemaVersion,
    /// The TS flags are not the caller's, or a bit above bit 2 is set (S5).
    TsFlags,
    /// The reserved header field is not 0.
    BadHeader,
    /// A file entry's path or record range does not follow the previous entry, a part is
    /// left over, `has_type_errors` is not 0 or 1, or a reserved byte is not 0.
    BadFileTable,
    /// A path is empty, not UTF-8, or has a NUL.
    BadPath,
    /// The paths are not in strictly ascending byte order (a path is there twice, or out of
    /// order).
    UnsortedFiles,
    /// A class or a field names a file that is not in the table, a class's field range does
    /// not follow the previous class, a field is left over, or a class has more fields than
    /// slots.
    BadClassTable,
    /// A record has an unknown code, a reserved flag bit, start after end, a reference out of
    /// range, a slot without a class or on a kind that is not a member access, or a value in
    /// a field that its null value turns off.
    BadRecord,
    /// The records of a file are not in strictly ascending (start, end, kind) order.
    UnsortedRecords,
    /// The file is not in the file table.
    NotInTable,
    /// tsrs found type errors in the file (S3).
    TypeErrors,
    /// The decoded-text hash differs from the caller's (S3).
    HashMismatch,
}

impl DropReason {
    /// Every reason, in [`DropReason::index`] order.
    pub const ALL: [DropReason; 16] = [
        DropReason::Truncated,
        DropReason::TrailingBytes,
        DropReason::BadMagic,
        DropReason::SchemaVersion,
        DropReason::FactSchemaVersion,
        DropReason::TsFlags,
        DropReason::BadHeader,
        DropReason::BadFileTable,
        DropReason::BadPath,
        DropReason::UnsortedFiles,
        DropReason::BadClassTable,
        DropReason::BadRecord,
        DropReason::UnsortedRecords,
        DropReason::NotInTable,
        DropReason::TypeErrors,
        DropReason::HashMismatch,
    ];

    /// The position of the reason in [`DropReason::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The stable name of the reason (for reports).
    pub const fn name(self) -> &'static str {
        match self {
            DropReason::Truncated => "truncated",
            DropReason::TrailingBytes => "trailing-bytes",
            DropReason::BadMagic => "bad-magic",
            DropReason::SchemaVersion => "schema-version",
            DropReason::FactSchemaVersion => "fact-schema-version",
            DropReason::TsFlags => "ts-flags",
            DropReason::BadHeader => "bad-header",
            DropReason::BadFileTable => "bad-file-table",
            DropReason::BadPath => "bad-path",
            DropReason::UnsortedFiles => "unsorted-files",
            DropReason::BadClassTable => "bad-class-table",
            DropReason::BadRecord => "bad-record",
            DropReason::UnsortedRecords => "unsorted-records",
            DropReason::NotInTable => "not-in-table",
            DropReason::TypeErrors => "type-errors",
            DropReason::HashMismatch => "hash-mismatch",
        }
    }
}

impl std::fmt::Display for DropReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::error::Error for DropReason {}

/// A count per [`DropReason`] (the join reports them, K15).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropCounts {
    counts: [u64; DropReason::ALL.len()],
}

impl DropCounts {
    /// Counts one drop.
    pub fn add(&mut self, reason: DropReason) {
        let c = &mut self.counts[reason.index()];
        *c = c.saturating_add(1);
    }

    /// The count of one reason.
    pub fn get(&self, reason: DropReason) -> u64 {
        self.counts[reason.index()]
    }

    /// The count of all reasons.
    pub fn total(&self) -> u64 {
        self.counts.iter().fold(0u64, |a, c| a.saturating_add(*c))
    }

    /// Each reason with its count, in [`DropReason::ALL`] order.
    pub fn iter(&self) -> impl Iterator<Item = (DropReason, u64)> + '_ {
        DropReason::ALL.iter().map(|r| (*r, self.get(*r)))
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn bytes_at<const N: usize>(b: &[u8], at: usize) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(&b[at..at + N]);
    out
}

/// Takes `count` entries of `size` bytes from the front of `rest`. The caller has checked
/// the total size, so this fails only on a bug; it still gives a drop, not a panic.
fn take<'a>(rest: &mut &'a [u8], count: u32, size: usize) -> Result<&'a [u8], DropReason> {
    let len = (count as usize)
        .checked_mul(size)
        .ok_or(DropReason::Truncated)?;
    let (part, tail) = rest.split_at_checked(len).ok_or(DropReason::Truncated)?;
    *rest = tail;
    Ok(part)
}

/// Reads a `.ep9h` file. Any mismatch or damage gives one [`DropReason`] and no hints.
///
/// The checks in order: the header size, the magic, the schema version, the fact-schema
/// version and the TS flags (S5), the reserved field, the total size, then the file table,
/// the class table and the records, entry by entry.
pub fn read(bytes: &[u8], expect: &Expect) -> Result<Hints, DropReason> {
    let head = bytes.get(..HEADER_SIZE).ok_or(DropReason::Truncated)?;
    if head[o::MAGIC..o::MAGIC + 8] != MAGIC {
        return Err(DropReason::BadMagic);
    }
    if u32_at(head, o::SCHEMA_VERSION) != HINT_SCHEMA_VERSION {
        return Err(DropReason::SchemaVersion);
    }
    let fact_schema_version = u32_at(head, o::FACT_SCHEMA_VERSION);
    if fact_schema_version != expect.fact_schema_version {
        return Err(DropReason::FactSchemaVersion);
    }
    let ts_flags = TsFlags::from_bits(u32_at(head, o::TS_FLAGS))
        .filter(|f| *f == expect.ts_flags)
        .ok_or(DropReason::TsFlags)?;
    if u32_at(head, o::HEADER_RESERVED) != 0 {
        return Err(DropReason::BadHeader);
    }
    let header = Header {
        tsrs_commit: bytes_at(head, o::TSRS_COMMIT),
        fact_schema_version,
        ts_flags,
    };
    let file_count = u32_at(head, o::FILE_COUNT);
    let class_count = u32_at(head, o::CLASS_COUNT);
    let field_count = u32_at(head, o::FIELD_COUNT);
    let record_count = u32_at(head, o::RECORD_COUNT);
    let string_bytes = u32_at(head, o::STRING_BYTES);

    // Each term is below 2^38, so the u64 sum cannot overflow.
    let total = HEADER_SIZE as u64
        + u64::from(file_count) * FILE_ENTRY_SIZE as u64
        + u64::from(class_count) * CLASS_ENTRY_SIZE as u64
        + u64::from(field_count) * FIELD_SIZE as u64
        + u64::from(record_count) * RECORD_SIZE as u64
        + u64::from(string_bytes);
    let len = bytes.len() as u64;
    if len < total {
        return Err(DropReason::Truncated);
    }
    if len > total {
        return Err(DropReason::TrailingBytes);
    }

    // From here every part is in the input, so each vector has at most one entry per input
    // entry (a Record is 48 B in memory per 32 B in the input).
    let mut rest = &bytes[HEADER_SIZE..];
    let file_part = take(&mut rest, file_count, FILE_ENTRY_SIZE)?;
    let class_part = take(&mut rest, class_count, CLASS_ENTRY_SIZE)?;
    let field_part = take(&mut rest, field_count, FIELD_SIZE)?;
    let record_part = take(&mut rest, record_count, RECORD_SIZE)?;
    let strings = take(&mut rest, string_bytes, 1)?;

    let files = read_files(file_part, strings, record_count)?;
    let classes = read_classes(class_part, file_count, field_count)?;
    let fields = read_fields(field_part, file_count)?;
    let records = read_records(record_part, &files, &classes)?;
    Ok(Hints {
        header,
        files,
        classes,
        fields,
        records,
    })
}

fn read_files(
    part: &[u8],
    strings: &[u8],
    record_count: u32,
) -> Result<Vec<FileEntry>, DropReason> {
    let mut files = Vec::with_capacity(part.len() / FILE_ENTRY_SIZE);
    let mut next_string = 0u32;
    let mut next_record = 0u32;
    let mut previous: Option<&[u8]> = None;
    for e in part.as_chunks::<FILE_ENTRY_SIZE>().0 {
        let path_offset = u32_at(e, o::FILE_PATH_OFFSET);
        let path_len = u32_at(e, o::FILE_PATH_LEN);
        let first_record = u32_at(e, o::FILE_FIRST_RECORD);
        let count = u32_at(e, o::FILE_RECORD_COUNT);
        let has_type_errors = match e[o::FILE_HAS_TYPE_ERRORS] {
            0 => false,
            1 => true,
            _ => return Err(DropReason::BadFileTable),
        };
        if path_offset != next_string
            || first_record != next_record
            || e[o::FILE_RESERVED..FILE_ENTRY_SIZE] != [0, 0, 0]
        {
            return Err(DropReason::BadFileTable);
        }
        next_string = path_offset
            .checked_add(path_len)
            .ok_or(DropReason::BadFileTable)?;
        next_record = first_record
            .checked_add(count)
            .filter(|n| *n <= record_count)
            .ok_or(DropReason::BadFileTable)?;
        let path_bytes = strings
            .get(path_offset as usize..next_string as usize)
            .ok_or(DropReason::BadFileTable)?;
        let path = std::str::from_utf8(path_bytes).map_err(|_| DropReason::BadPath)?;
        if path.is_empty() || path.contains('\0') {
            return Err(DropReason::BadPath);
        }
        if previous.is_some_and(|p| p >= path_bytes) {
            return Err(DropReason::UnsortedFiles);
        }
        previous = Some(path_bytes);
        files.push(FileEntry {
            path: path.to_owned(),
            text_hash: bytes_at(e, o::FILE_TEXT_HASH),
            has_type_errors,
            first_record,
            record_count: count,
        });
    }
    // The strings part holds the paths and nothing else (S6); every record has a file.
    if next_string as usize != strings.len() || next_record != record_count {
        return Err(DropReason::BadFileTable);
    }
    Ok(files)
}

fn read_classes(
    part: &[u8],
    file_count: u32,
    field_count: u32,
) -> Result<Vec<ClassEntry>, DropReason> {
    let mut classes = Vec::with_capacity(part.len() / CLASS_ENTRY_SIZE);
    let mut next_field = 0u32;
    for e in part.as_chunks::<CLASS_ENTRY_SIZE>().0 {
        let class = ClassEntry {
            decl: DeclRef {
                file: u32_at(e, o::CLASS_DECL_FILE),
                offset: u32_at(e, o::CLASS_DECL_OFFSET),
            },
            slot_count: u32_at(e, o::CLASS_SLOT_COUNT),
            first_field: u32_at(e, o::CLASS_FIRST_FIELD),
            field_count: u32_at(e, o::CLASS_FIELD_COUNT),
        };
        if class.decl.file >= file_count
            || class.first_field != next_field
            || class.field_count > class.slot_count
        {
            return Err(DropReason::BadClassTable);
        }
        next_field = next_field
            .checked_add(class.field_count)
            .filter(|n| *n <= field_count)
            .ok_or(DropReason::BadClassTable)?;
        classes.push(class);
    }
    if next_field != field_count {
        return Err(DropReason::BadClassTable);
    }
    Ok(classes)
}

fn read_fields(part: &[u8], file_count: u32) -> Result<Vec<DeclRef>, DropReason> {
    let mut fields = Vec::with_capacity(part.len() / FIELD_SIZE);
    for e in part.as_chunks::<FIELD_SIZE>().0 {
        let field = DeclRef {
            file: u32_at(e, o::FIELD_FILE),
            offset: u32_at(e, o::FIELD_OFFSET),
        };
        if field.file >= file_count {
            return Err(DropReason::BadClassTable);
        }
        fields.push(field);
    }
    Ok(fields)
}

fn read_records(
    part: &[u8],
    files: &[FileEntry],
    classes: &[ClassEntry],
) -> Result<Vec<Record>, DropReason> {
    let mut records = Vec::with_capacity(part.len() / RECORD_SIZE);
    let mut rest = part;
    for file in files {
        let mut previous: Option<(u32, u32, u16)> = None;
        for e in take(&mut rest, file.record_count, RECORD_SIZE)?
            .as_chunks::<RECORD_SIZE>()
            .0
        {
            let record = read_record(e, files.len(), classes)?;
            let key = record.key();
            if previous.is_some_and(|p| p >= key) {
                return Err(DropReason::UnsortedRecords);
            }
            previous = Some(key);
            records.push(record);
        }
    }
    Ok(records)
}

fn read_record(e: &[u8], file_count: usize, classes: &[ClassEntry]) -> Result<Record, DropReason> {
    let bad = DropReason::BadRecord;
    let start = u32_at(e, o::REC_START);
    let end = u32_at(e, o::REC_END);
    let kind = NodeKind::from_code(u16_at(e, o::REC_KIND)).ok_or(bad)?;
    let repr = Repr::from_code(e[o::REC_REPR]).ok_or(bad)?;
    let element = Repr::from_code(e[o::REC_ELEMENT]).ok_or(bad)?;
    let flags = e[o::REC_FLAGS];
    if flags & !0b111 != 0 || start > end {
        return Err(bad);
    }
    let class = match u32_at(e, o::REC_CLASS) {
        NULL_U32 => None,
        c => Some(c),
    };
    let class_entry = match class {
        None => None,
        Some(c) => Some(classes.get(c as usize).ok_or(bad)?),
    };
    let member = matches!(
        kind,
        NodeKind::StaticMemberExpression
            | NodeKind::ComputedMemberExpression
            | NodeKind::PrivateFieldExpression
    );
    // A1H-R4 option A: on a kind that is not a member access the class id is the class of the
    // value, so it needs repr ClassInstance. A member kind keeps any repr (the receiver class).
    if !member && class.is_some() && repr != Repr::ClassInstance {
        return Err(bad);
    }
    let slot = match u16_at(e, o::REC_SLOT) {
        NULL_U16 => None,
        s => match class_entry {
            Some(k) if member && u32::from(s) < k.slot_count => Some(s),
            _ => return Err(bad),
        },
    };
    let target_offset = u32_at(e, o::REC_TARGET_OFFSET);
    let call_target = match u32_at(e, o::REC_TARGET_FILE) {
        NULL_U32 if target_offset == 0 => None,
        f if f != NULL_U32 && (f as usize) < file_count => Some(DeclRef {
            file: f,
            offset: target_offset,
        }),
        _ => return Err(bad),
    };
    let enum_value = u32_at(e, o::REC_ENUM_CONST) as i32;
    let enum_const = match flags & 4 {
        0 if enum_value == 0 => None,
        0 => return Err(bad),
        _ => Some(enum_value),
    };
    Ok(Record {
        start,
        end,
        kind,
        repr,
        element,
        non_null: flags & 1 != 0,
        dts_lib: flags & 2 != 0,
        enum_const,
        confidence: e[o::REC_CONFIDENCE],
        class,
        slot,
        call_target,
    })
}
