//! Byte offset to (line, UTF-16 column) mapping, shared by early errors and the `CompiledUnit`
//! source position table (01-front-end §5: "position tables: UTF-8 offsets; the UTF-16 column map
//! is built lazily").
//!
//! ```text
//! source bytes (UTF-8, or WTF-8 with lone surrogates as 3-byte ED A0..BF xx sequences)
//!   |  LineMap::new: one pass
//!   v
//! line_starts: [0, 12, 40, ...]            byte offset of each line start
//! wide:        [(start, len, extra_before)]   one entry per non-ASCII character
//!   |
//!   v  position(offset): 2 binary searches, O(log n), no rescan of long (minified) lines
//! Position { line, column }                 column in UTF-16 code units, both 1-based
//! ```
//!
//! UTF-16 units per encoded character, from its lead byte:
//!
//! | Bytes | Characters | UTF-16 units |
//! |---|---|---|
//! | 1 | U+0000-U+007F | 1 |
//! | 2 | U+0080-U+07FF | 1 |
//! | 3 | U+0800-U+FFFF, incl. lone surrogates U+D800-U+DFFF (WTF-8) and the U+FFFD shim | 1 |
//! | 4 | U+10000-U+10FFFF (astral) | 2 |

/// A source position as stack traces print it (`file:line:col`).
///
/// - `line` starts at 1. LF, CR, CRLF, U+2028 and U+2029 end a line (ECMA-262 LineTerminatorSequence).
/// - `column` starts at 1 and counts UTF-16 code units, as JavaScript string indices do.
///   A character outside the BMP counts 2. A lone surrogate counts 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Position {
    /// 1-based line number.
    pub line: u32,
    /// 1-based column in UTF-16 code units.
    pub column: u32,
}

/// Line starts and UTF-16 widths of one source text. Build it once per source; query it often.
///
/// Offsets are `u32`, as in oxc spans. A source longer than `u32::MAX` bytes is not supported:
/// oxc rejects it first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineMap {
    len: u32,
    line_starts: Vec<u32>,
    /// Start offset of each non-ASCII character, in order.
    wide_starts: Vec<u32>,
    /// Encoded length (2-4) of each non-ASCII character.
    wide_lens: Vec<u8>,
    /// `bytes - utf16_units` summed over the characters before each non-ASCII character.
    wide_extra_before: Vec<u32>,
}

impl LineMap {
    /// Index a UTF-8 or WTF-8 source. Invalid bytes do not panic: a byte that cannot start a
    /// sequence counts as one character of one UTF-16 unit.
    ///
    /// # Panics
    ///
    /// Panics if `source` is longer than `u32::MAX` bytes.
    #[must_use]
    pub fn new(source: &[u8]) -> Self {
        let len = u32::try_from(source.len()).expect("sources are at most u32::MAX bytes");
        let mut map = Self {
            len,
            line_starts: vec![0],
            wide_starts: Vec::new(),
            wide_lens: Vec::new(),
            wide_extra_before: Vec::new(),
        };
        let mut extra = 0u32;
        let mut i = 0usize;
        while i < source.len() {
            let byte = source[i];
            if byte < 0x80 {
                match byte {
                    b'\n' => map.line_starts.push(offset(i + 1)),
                    // CRLF is one terminator: the LF records the start.
                    b'\r' if source.get(i + 1) != Some(&b'\n') => {
                        map.line_starts.push(offset(i + 1));
                    }
                    _ => {}
                }
                i += 1;
                continue;
            }
            let (full_len, units) = encoded_len(byte);
            // A truncated sequence at the end counts the bytes that exist, as 1 unit (as U+FFFD).
            let char_len = full_len.min(source.len() - i);
            let units = if char_len < full_len { 1 } else { units };
            map.wide_starts.push(offset(i));
            map.wide_lens
                .push(u8::try_from(char_len).expect("at most 4"));
            map.wide_extra_before.push(extra);
            extra += offset(char_len) - units;
            // U+2028 and U+2029 are E2 80 A8 and E2 80 A9.
            if byte == 0xE2
                && source.get(i + 1) == Some(&0x80)
                && matches!(source.get(i + 2), Some(0xA8 | 0xA9))
            {
                map.line_starts.push(offset(i + 3));
            }
            i += char_len;
        }
        map
    }

    /// Length of the indexed source in bytes.
    #[must_use]
    pub fn len(&self) -> u32 {
        self.len
    }

    /// True for an empty source.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Number of lines (at least 1).
    #[must_use]
    pub fn line_count(&self) -> u32 {
        offset(self.line_starts.len())
    }

    /// The UTF-16 index of byte `offset`, as `String.prototype` methods count it.
    ///
    /// An offset inside a multi-byte character moves back to the character start. An offset past
    /// the end moves to the end.
    #[must_use]
    pub fn utf16_offset(&self, offset: u32) -> u32 {
        let offset = offset.min(self.len);
        // The last non-ASCII character that starts at or before `offset`.
        let n = self.wide_starts.partition_point(|&start| start <= offset);
        if n == 0 {
            return offset;
        }
        let k = n - 1;
        let start = self.wide_starts[k];
        let char_len = u32::from(self.wide_lens[k]);
        if offset < start + char_len {
            // Inside (or at the start of) character k.
            start - self.wide_extra_before[k]
        } else {
            let (_, units) = units_of_len(char_len);
            offset - (self.wide_extra_before[k] + char_len - units)
        }
    }

    /// Line and UTF-16 column of byte `offset`. Same clamping as [`LineMap::utf16_offset`].
    #[must_use]
    pub fn position(&self, offset: u32) -> Position {
        let offset = offset.min(self.len);
        // `line_starts[0] == 0`, so the partition point is at least 1.
        let line = self.line_starts.partition_point(|&start| start <= offset);
        let line_start = self.line_starts[line - 1];
        let column = self.utf16_offset(offset) - self.utf16_offset(line_start) + 1;
        Position {
            line: offset_u32(line),
            column,
        }
    }
}

/// Encoded length and UTF-16 units from a non-ASCII lead byte.
fn encoded_len(lead: u8) -> (usize, u32) {
    match lead {
        0xC0..=0xDF => (2, 1),
        0xE0..=0xEF => (3, 1),
        0xF0..=0xF7 => (4, 2),
        // A continuation byte or an invalid lead byte on its own.
        _ => (1, 1),
    }
}

fn units_of_len(char_len: u32) -> (u32, u32) {
    match char_len {
        4 => (4, 2),
        n => (n, 1),
    }
}

fn offset(n: usize) -> u32 {
    u32::try_from(n).expect("sources are at most u32::MAX bytes")
}

fn offset_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(src: &str, offset: u32) -> (u32, u32) {
        pos_bytes(src.as_bytes(), offset)
    }

    fn pos_bytes(src: &[u8], offset: u32) -> (u32, u32) {
        let p = LineMap::new(src).position(offset);
        (p.line, p.column)
    }

    /// Reference: decode with `char_indices` and count UTF-16 units (valid UTF-8 only).
    fn reference(src: &str, offset: u32) -> (u32, u32) {
        let mut off = (offset as usize).min(src.len());
        while !src.is_char_boundary(off) {
            off -= 1;
        }
        let mut line = 1;
        let mut col = 1;
        for (at, c) in src[..off].char_indices() {
            match c {
                '\n' | '\u{2028}' | '\u{2029}' => {
                    line += 1;
                    col = 1;
                }
                '\r' => {
                    // Look at the whole source: a CR before an LF is the first half of a CRLF.
                    if src.as_bytes().get(at + 1) == Some(&b'\n') {
                        col += 1; // the CR of a CRLF is still on this line
                    } else {
                        line += 1;
                        col = 1;
                    }
                }
                c => col += u32::try_from(c.len_utf16()).unwrap(),
            }
        }
        (line, col)
    }

    /// `LineMap` and the `CompiledUnit` position table reader agree on every offset of valid
    /// UTF-8 text (lone surrogates cannot be in a `&str`; the WTF-8 tests cover them).
    #[test]
    fn agrees_with_the_compiled_unit_source_lines() {
        let mut sources: Vec<String> = [
            "",
            "a\nb\r\nc\rd\u{2028}e\u{2029}f",
            "\r\n\r\n\n\r",
            "x = '\u{1F600}\u{E9}\u{4E2D}';\n\t\u{1F600}y",
            "// \u{2028} in a comment\nz",
            "\u{FEFF}bom",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        // A pseudo-random mix of 1-4 byte characters and every line terminator.
        let pool = [
            'a',
            '\n',
            '\r',
            '\u{E9}',
            '\u{2028}',
            '\u{2029}',
            '\u{4E2D}',
            '\u{1F600}',
            ' ',
        ];
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut mix = String::new();
        for _ in 0..2_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            mix.push(pool[usize::try_from(seed % pool.len() as u64).unwrap()]);
        }
        sources.push(mix);
        for src in &sources {
            let ours = LineMap::new(src.as_bytes());
            let theirs = ep9_bytecode::SourceLines::new(src);
            for offset in 0..=u32::try_from(src.len()).unwrap() + 2 {
                let a = ours.position(offset);
                let b = theirs.line_column(offset);
                assert_eq!(
                    (a.line, a.column),
                    (b.line, b.column),
                    "offset {offset} in {src:?}"
                );
            }
        }
    }

    #[test]
    fn first_line() {
        assert_eq!(pos("abc", 0), (1, 1));
        assert_eq!(pos("abc", 2), (1, 3));
        assert_eq!(pos("abc", 3), (1, 4));
        assert_eq!(pos("", 0), (1, 1));
        assert!(LineMap::new(b"").is_empty());
        assert_eq!(LineMap::new(b"").line_count(), 1);
    }

    #[test]
    fn every_line_terminator() {
        // a LF b CRLF c CR d LS e PS f
        let src = "a\nb\r\nc\rd\u{2028}e\u{2029}f";
        let at = |c: char| u32::try_from(src.find(c).unwrap()).unwrap();
        assert_eq!(pos(src, at('a')), (1, 1));
        assert_eq!(pos(src, at('b')), (2, 1));
        assert_eq!(pos(src, at('c')), (3, 1));
        assert_eq!(pos(src, at('d')), (4, 1));
        assert_eq!(pos(src, at('e')), (5, 1));
        assert_eq!(pos(src, at('f')), (6, 1));
        // The CR of a CRLF is still on line 2.
        assert_eq!(pos(src, at('\r')), (2, 2));
        assert_eq!(LineMap::new(src.as_bytes()).line_count(), 6);
    }

    #[test]
    fn utf16_columns_for_bmp_and_astral() {
        // U+00E9 is 2 bytes and 1 UTF-16 unit; U+1F600 is 4 bytes and 2 units.
        let src = "x\n\u{e9}\u{1F600}z";
        let z = u32::try_from(src.find('z').unwrap()).unwrap();
        assert_eq!(z, 8);
        assert_eq!(pos(src, z), (2, 4));
        let map = LineMap::new(src.as_bytes());
        // x \n e-acute emoji(2) z
        assert_eq!(map.utf16_offset(z), 5);
        assert_eq!(map.utf16_offset(u32::try_from(src.len()).unwrap()), 6);
    }

    #[test]
    fn lone_surrogates_in_wtf8_count_one_unit() {
        // `"` U+D800 `"` `;` `x`: WTF-8 encodes the lone lead surrogate as ED A0 80.
        let src: &[u8] = b"\"\xED\xA0\x80\";x";
        assert_eq!(pos_bytes(src, 5), (1, 4)); // `;`
        assert_eq!(pos_bytes(src, 6), (1, 5)); // `x`
        // A lone trail surrogate U+DC00 (ED B0 80) and a pair written as two lone halves.
        let src: &[u8] = b"\xED\xB0\x80\xED\xA0\xBD\xED\xB8\x80y";
        assert_eq!(pos_bytes(src, 9), (1, 4));
        // The U+FFFD shim (07-runtime §7) has the same widths: 3 bytes, 1 unit.
        let shim = "\"\u{FFFD}\";x";
        assert_eq!(pos(shim, 6), pos_bytes(b"\"\xED\xA0\x80\";x", 6));
    }

    #[test]
    fn offsets_clamp_to_char_boundaries() {
        let src = "\u{1F600}a";
        // Offsets 1..=3 are inside the emoji: they map to its start.
        assert_eq!(pos(src, 1), (1, 1));
        assert_eq!(pos(src, 2), (1, 1));
        assert_eq!(pos(src, 3), (1, 1));
        assert_eq!(pos(src, 4), (1, 3));
        assert_eq!(pos(src, 99), (1, 4));
    }

    #[test]
    fn invalid_bytes_do_not_panic() {
        // A stray continuation byte (1 unit), a truncated 4-byte sequence at the end (1 unit).
        let src: &[u8] = b"a\x80b\xF0\x9F";
        assert_eq!(pos_bytes(src, 2), (1, 3));
        assert_eq!(pos_bytes(src, 4), (1, 4)); // inside the truncated sequence: its start
        assert_eq!(pos_bytes(src, 5), (1, 5));
    }

    #[test]
    fn matches_a_char_by_char_reference_on_every_offset() {
        let src = "let s = '\u{e9}t\u{e9}';\r\n// \u{1F600}\u{1F600} \u{2028}x = `\u{4e2d}\n\u{10FFFF}`;\rend";
        let map = LineMap::new(src.as_bytes());
        for offset in 0..=u32::try_from(src.len()).unwrap() + 2 {
            let p = map.position(offset);
            assert_eq!(
                (p.line, p.column),
                reference(src, offset),
                "offset {offset}"
            );
        }
    }

    #[test]
    fn long_minified_line_is_fast_enough() {
        // One 4 MiB line with a non-ASCII char every 64 bytes: lookups use binary search.
        let unit = format!("{}\u{e9}", "a".repeat(62));
        let src = unit.repeat(65_536);
        let map = LineMap::new(src.as_bytes());
        let last = u32::try_from(src.len()).unwrap();
        // 64 bytes = 63 units per repetition.
        assert_eq!(map.position(last).column, 63 * 65_536 + 1);
        for k in (0..last).step_by(4093) {
            let _ = map.position(k);
        }
    }
}
