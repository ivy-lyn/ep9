//! Byte offset to (line, UTF-16 column) mapping.

/// A source position as stack traces print it (`file:line:col`).
///
/// - `line` starts at 1. LF, CR, CRLF, U+2028 and U+2029 end a line (ECMA-262 LineTerminatorSequence).
/// - `column` starts at 1 and counts UTF-16 code units, as JavaScript string indices do.
///   A character outside the BMP counts 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Position {
    /// 1-based line number.
    pub line: u32,
    /// 1-based column in UTF-16 code units.
    pub column: u32,
}

/// Byte offsets of each line start. Built once per source that has errors.
pub(crate) struct LineIndex<'s> {
    source: &'s str,
    starts: Vec<usize>,
}

impl<'s> LineIndex<'s> {
    pub(crate) fn new(source: &'s str) -> Self {
        let mut starts = vec![0];
        let bytes = source.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => starts.push(i + 1),
                // CRLF is one line terminator: the LF arm records the start.
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

    /// Position of the byte `offset`. An offset past the end or inside a UTF-8 sequence moves
    /// back to the previous character boundary.
    pub(crate) fn position(&self, offset: u32) -> Position {
        let mut offset = (offset as usize).min(self.source.len());
        while !self.source.is_char_boundary(offset) {
            offset -= 1;
        }
        // `starts[0] == 0`, so the partition point is at least 1.
        let line = self.starts.partition_point(|&start| start <= offset);
        let line_start = self.starts[line - 1];
        let units: usize = self.source[line_start..offset]
            .chars()
            .map(char::len_utf16)
            .sum();
        Position {
            line: saturate(line),
            column: saturate(units + 1),
        }
    }
}

fn saturate(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(src: &str, offset: u32) -> (u32, u32) {
        let p = LineIndex::new(src).position(offset);
        (p.line, p.column)
    }

    #[test]
    fn first_line() {
        assert_eq!(pos("abc", 0), (1, 1));
        assert_eq!(pos("abc", 2), (1, 3));
        assert_eq!(pos("abc", 3), (1, 4));
        assert_eq!(pos("", 0), (1, 1));
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
    }

    #[test]
    fn utf16_columns() {
        // U+00E9 is 2 bytes and 1 UTF-16 unit; U+1F600 is 4 bytes and 2 units.
        let src = "x\n\u{e9}\u{1F600}z";
        let z = u32::try_from(src.find('z').unwrap()).unwrap();
        assert_eq!(z, 8);
        assert_eq!(pos(src, z), (2, 4));
    }

    #[test]
    fn offsets_clamp_to_char_boundaries() {
        let src = "\u{1F600}a";
        // Offsets 1..=3 are inside the emoji: they map to its start.
        assert_eq!(pos(src, 2), (1, 1));
        assert_eq!(pos(src, 4), (1, 3));
        assert_eq!(pos(src, 99), (1, 4));
    }
}
