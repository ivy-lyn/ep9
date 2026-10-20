//! Source positions: the serialized position table, lookup, and the lazy byte offset ->
//! (line, UTF-16 column) map.
//!
//! The table stores UTF-8 byte offsets (01-front-end §5). The UTF-16 column is computed only
//! when needed (`Error.stack`, the inspector), from the source text, by [`SourceLines`].
//!
//! Serialized form (one entry after another, all integers unsigned LEB128):
//!
//! ```text
//! entry := uleb(bytecode_offset - prev_bytecode_offset)
//!          uleb(zigzag(source_offset - prev_source_offset) << 1 | is_statement)
//! prev starts at (0, 0). After the first entry the bytecode delta is > 0.
//! ```
//!
//! LEB128 must be minimal: the reader rejects a redundant final 0x00 byte, so each table has
//! exactly one encoding.

use crate::unit::PositionEntry;

/// Appends `v` as unsigned LEB128.
pub(crate) fn write_uleb(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Reads a minimal unsigned LEB128 at `*at`.
pub(crate) fn read_uleb(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut v: u64 = 0;
    let mut shift = 0u32;
    loop {
        let byte = *bytes.get(*at)?;
        *at += 1;
        let low = u64::from(byte & 0x7f);
        if shift == 63 && low > 1 {
            return None; // overflow
        }
        v |= low << shift;
        if byte & 0x80 == 0 {
            // Minimal form: a multi-byte number never ends in 0x00.
            if byte == 0 && shift > 0 {
                return None;
            }
            return Some(v);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

/// Encodes a position table. Entries must have strictly increasing bytecode offsets.
pub fn encode_positions(entries: &[PositionEntry]) -> Vec<u8> {
    let mut out = Vec::with_capacity(entries.len() * 3);
    let (mut prev_off, mut prev_src) = (0u32, 0u32);
    for e in entries {
        debug_assert!(e.bytecode_offset >= prev_off);
        write_uleb(&mut out, u64::from(e.bytecode_offset - prev_off));
        let delta = i64::from(e.source_offset) - i64::from(prev_src);
        write_uleb(&mut out, (zigzag(delta) << 1) | u64::from(e.is_statement));
        prev_off = e.bytecode_offset;
        prev_src = e.source_offset;
    }
    out
}

/// Why a serialized position table is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionTableError {
    /// Truncated or non-minimal LEB128, or a value out of range.
    Malformed,
    /// Bytecode offsets are not strictly increasing.
    NotIncreasing,
}

/// Decodes a position table.
pub fn decode_positions(bytes: &[u8]) -> Result<Vec<PositionEntry>, PositionTableError> {
    let mut out = Vec::new();
    let mut at = 0usize;
    let (mut off, mut src) = (0u32, 0u32);
    while at < bytes.len() {
        let d_off = read_uleb(bytes, &mut at).ok_or(PositionTableError::Malformed)?;
        let packed = read_uleb(bytes, &mut at).ok_or(PositionTableError::Malformed)?;
        if !out.is_empty() && d_off == 0 {
            return Err(PositionTableError::NotIncreasing);
        }
        let d_off = u32::try_from(d_off).map_err(|_| PositionTableError::Malformed)?;
        off = off
            .checked_add(d_off)
            .ok_or(PositionTableError::Malformed)?;
        let d_src = unzigzag(packed >> 1);
        let s = i64::from(src)
            .checked_add(d_src)
            .ok_or(PositionTableError::Malformed)?;
        src = u32::try_from(s).map_err(|_| PositionTableError::Malformed)?;
        out.push(PositionEntry {
            bytecode_offset: off,
            source_offset: src,
            is_statement: packed & 1 == 1,
        });
    }
    Ok(out)
}

/// The entry that covers the instruction at `bytecode_offset`: the last entry at or before it.
pub fn position_at(entries: &[PositionEntry], bytecode_offset: u32) -> Option<PositionEntry> {
    let i = entries.partition_point(|e| e.bytecode_offset <= bytecode_offset);
    if i == 0 { None } else { Some(entries[i - 1]) }
}

/// A source position as stack traces print it (`file:line:col`).
///
/// `line` starts at 1; LF, CR, CRLF, U+2028 and U+2029 end a line. `column` starts at 1 and
/// counts UTF-16 code units, so a character outside the BMP counts 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LineColumn {
    /// 1-based line.
    pub line: u32,
    /// 1-based UTF-16 column.
    pub column: u32,
}

/// Byte offsets of each line start of a source text. Build it lazily, once per source.
#[derive(Clone, Debug)]
pub struct SourceLines<'s> {
    source: &'s str,
    starts: Vec<usize>,
}

impl<'s> SourceLines<'s> {
    /// Scans `source` for line terminators.
    pub fn new(source: &'s str) -> Self {
        let mut starts = vec![0];
        let bytes = source.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => starts.push(i + 1),
                // CRLF is one terminator: the LF arm records the line start.
                b'\r' if bytes.get(i + 1) != Some(&b'\n') => starts.push(i + 1),
                // U+2028 and U+2029 are E2 80 A8 and E2 80 A9 in UTF-8.
                0xE2 if bytes.get(i + 1) == Some(&0x80)
                    && matches!(bytes.get(i + 2), Some(0xA8 | 0xA9)) =>
                {
                    i += 2;
                    starts.push(i + 1);
                }
                _ => {}
            }
            i += 1;
        }
        Self { source, starts }
    }

    /// Line and UTF-16 column of byte `offset`. An offset past the end, or inside a UTF-8
    /// sequence, moves back to the previous character boundary.
    pub fn line_column(&self, offset: u32) -> LineColumn {
        let mut offset = (offset as usize).min(self.source.len());
        while !self.source.is_char_boundary(offset) {
            offset -= 1;
        }
        // starts[0] == 0, so the partition point is at least 1.
        let line = self.starts.partition_point(|&s| s <= offset);
        let line_start = self.starts[line - 1];
        let units: usize = self.source[line_start..offset]
            .chars()
            .map(char::len_utf16)
            .sum();
        LineColumn {
            line: u32::try_from(line).unwrap_or(u32::MAX),
            column: u32::try_from(units + 1).unwrap_or(u32::MAX),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(off: u32, src: u32, stmt: bool) -> PositionEntry {
        PositionEntry {
            bytecode_offset: off,
            source_offset: src,
            is_statement: stmt,
        }
    }

    #[test]
    fn leb128_round_trip_and_minimal() {
        for v in [
            0u64,
            1,
            127,
            128,
            300,
            16383,
            16384,
            u32::MAX as u64,
            u64::MAX,
        ] {
            let mut b = Vec::new();
            write_uleb(&mut b, v);
            let mut at = 0;
            assert_eq!(read_uleb(&b, &mut at), Some(v));
            assert_eq!(at, b.len());
        }
        // Non-minimal forms of 0 and 1 are rejected.
        assert_eq!(read_uleb(&[0x80, 0x00], &mut 0), None);
        assert_eq!(read_uleb(&[0x81, 0x80, 0x00], &mut 0), None);
        // Truncated.
        assert_eq!(read_uleb(&[0x80], &mut 0), None);
        // 2^64 overflows.
        let mut b = vec![0xff; 9];
        b.push(0x02);
        assert_eq!(read_uleb(&b, &mut 0), None);
    }

    #[test]
    fn table_round_trip() {
        let t = vec![
            e(0, 10, true),
            e(2, 12, false),
            e(5, 3, true),
            e(9, u32::MAX, false),
            e(10, 0, true),
        ];
        let b = encode_positions(&t);
        assert_eq!(decode_positions(&b).unwrap(), t);
        assert_eq!(encode_positions(&decode_positions(&b).unwrap()), b);
        assert_eq!(decode_positions(&[]).unwrap(), vec![]);
    }

    #[test]
    fn table_rejects_bad_input() {
        // Two entries at the same offset.
        let b = encode_positions(&[e(4, 1, true), e(4, 2, true)]);
        assert_eq!(decode_positions(&b), Err(PositionTableError::NotIncreasing));
        // Source offset below 0.
        let mut b = Vec::new();
        write_uleb(&mut b, 0);
        write_uleb(&mut b, zigzag(-1) << 1);
        assert_eq!(decode_positions(&b), Err(PositionTableError::Malformed));
        // Odd number of numbers.
        assert_eq!(
            decode_positions(&[0x00]),
            Err(PositionTableError::Malformed)
        );
    }

    #[test]
    fn lookup() {
        let t = [e(0, 1, true), e(4, 7, false), e(9, 2, true)];
        assert_eq!(position_at(&t, 0), Some(t[0]));
        assert_eq!(position_at(&t, 3), Some(t[0]));
        assert_eq!(position_at(&t, 4), Some(t[1]));
        assert_eq!(position_at(&t, 100), Some(t[2]));
        assert_eq!(position_at(&t[1..], 0), None);
    }

    #[test]
    fn line_columns() {
        let src = "a\nb\r\nc\rd\u{2028}e\u{2029}f";
        let lines = SourceLines::new(src);
        let at = |c: char| src.find(c).unwrap() as u32;
        let lc = |o| {
            let p = lines.line_column(o);
            (p.line, p.column)
        };
        assert_eq!(lc(at('a')), (1, 1));
        assert_eq!(lc(at('b')), (2, 1));
        assert_eq!(lc(at('c')), (3, 1));
        assert_eq!(lc(at('d')), (4, 1));
        assert_eq!(lc(at('e')), (5, 1));
        assert_eq!(lc(at('f')), (6, 1));
        // U+00E9 is 2 bytes, 1 unit; U+1F600 is 4 bytes, 2 units.
        let src = "x\n\u{e9}\u{1F600}z";
        let lines = SourceLines::new(src);
        let p = lines.line_column(src.find('z').unwrap() as u32);
        assert_eq!((p.line, p.column), (2, 4));
        // Inside a UTF-8 sequence: back to the character start. Past the end: clamped.
        let lines = SourceLines::new("\u{1F600}a");
        assert_eq!(lines.line_column(2).column, 1);
        assert_eq!(lines.line_column(99).column, 4);
    }
}
