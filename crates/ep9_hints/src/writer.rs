//! The `.ep9h` writer. It reads its own output back, so it never writes a file that
//! [`crate::read()`] drops or reads as other hints.

use crate::model::Hints;
use crate::offsets as o;
use crate::reader::{DropReason, Expect, read};
use crate::{HEADER_SIZE, HINT_SCHEMA_VERSION, MAGIC, NULL_U16, NULL_U32, RECORD_SIZE};

/// Why [`write()`] refused its input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteError {
    /// A count or the strings part does not fit in a `u32`.
    TooLarge,
    /// The reader would drop the output for this reason (for example, unsorted records).
    Invalid(DropReason),
    /// The reader would read other hints: an id equals its null value ([`crate::NULL_U32`],
    /// [`crate::NULL_U16`]).
    Lossy,
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::TooLarge => f.write_str("a count does not fit in a u32"),
            WriteError::Invalid(r) => write!(f, "the reader would drop the output: {r}"),
            WriteError::Lossy => f.write_str("an id equals its null value"),
        }
    }
}

impl std::error::Error for WriteError {}

fn count(n: usize) -> Result<u32, WriteError> {
    u32::try_from(n).map_err(|_| WriteError::TooLarge)
}

fn put_u32(out: &mut [u8], at: usize, v: u32) {
    out[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// Writes `hints` as a `.ep9h` file.
pub fn write(hints: &Hints) -> Result<Vec<u8>, WriteError> {
    let string_bytes = hints
        .files
        .iter()
        .try_fold(0u32, |a, f| a.checked_add(count(f.path.len()).ok()?))
        .ok_or(WriteError::TooLarge)?;

    let mut head = [0u8; HEADER_SIZE];
    head[o::MAGIC..o::MAGIC + 8].copy_from_slice(&MAGIC);
    put_u32(&mut head, o::SCHEMA_VERSION, HINT_SCHEMA_VERSION);
    put_u32(
        &mut head,
        o::FACT_SCHEMA_VERSION,
        hints.header.fact_schema_version,
    );
    put_u32(&mut head, o::TS_FLAGS, hints.header.ts_flags.to_bits());
    head[o::TSRS_COMMIT..o::TSRS_COMMIT + 20].copy_from_slice(&hints.header.tsrs_commit);
    put_u32(&mut head, o::FILE_COUNT, count(hints.files.len())?);
    put_u32(&mut head, o::CLASS_COUNT, count(hints.classes.len())?);
    put_u32(&mut head, o::FIELD_COUNT, count(hints.fields.len())?);
    put_u32(&mut head, o::RECORD_COUNT, count(hints.records.len())?);
    put_u32(&mut head, o::STRING_BYTES, string_bytes);

    let mut out = Vec::new();
    out.extend_from_slice(&head);

    let mut path_offset = 0u32;
    for f in &hints.files {
        let len = count(f.path.len())?;
        out.extend_from_slice(&path_offset.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&f.first_record.to_le_bytes());
        out.extend_from_slice(&f.record_count.to_le_bytes());
        out.extend_from_slice(&f.text_hash);
        out.extend_from_slice(&[u8::from(f.has_type_errors), 0, 0, 0]);
        path_offset = path_offset.checked_add(len).ok_or(WriteError::TooLarge)?;
    }
    for c in &hints.classes {
        for v in [
            c.decl.file,
            c.decl.offset,
            c.slot_count,
            c.first_field,
            c.field_count,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    for f in &hints.fields {
        out.extend_from_slice(&f.file.to_le_bytes());
        out.extend_from_slice(&f.offset.to_le_bytes());
    }
    for r in &hints.records {
        let mut e = [0u8; RECORD_SIZE];
        put_u32(&mut e, o::REC_START, r.start);
        put_u32(&mut e, o::REC_END, r.end);
        e[o::REC_KIND..o::REC_KIND + 2].copy_from_slice(&r.kind.code().to_le_bytes());
        e[o::REC_REPR] = r.repr.code();
        e[o::REC_ELEMENT] = r.element.code();
        e[o::REC_FLAGS] = u8::from(r.non_null)
            | (u8::from(r.dts_lib) << 1)
            | (u8::from(r.enum_const.is_some()) << 2);
        e[o::REC_CONFIDENCE] = r.confidence;
        e[o::REC_SLOT..o::REC_SLOT + 2].copy_from_slice(&r.slot.unwrap_or(NULL_U16).to_le_bytes());
        put_u32(&mut e, o::REC_CLASS, r.class.unwrap_or(NULL_U32));
        let (file, offset) = r.call_target.map_or((NULL_U32, 0), |d| (d.file, d.offset));
        put_u32(&mut e, o::REC_TARGET_FILE, file);
        put_u32(&mut e, o::REC_TARGET_OFFSET, offset);
        put_u32(&mut e, o::REC_ENUM_CONST, r.enum_const.unwrap_or(0) as u32);
        out.extend_from_slice(&e);
    }
    for f in &hints.files {
        out.extend_from_slice(f.path.as_bytes());
    }

    let expect = Expect {
        fact_schema_version: hints.header.fact_schema_version,
        ts_flags: hints.header.ts_flags,
    };
    let back = read(&out, &expect).map_err(WriteError::Invalid)?;
    if back != *hints {
        return Err(WriteError::Lossy);
    }
    Ok(out)
}
