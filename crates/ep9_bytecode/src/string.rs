//! Strings in a unit: Latin-1 or UTF-16, one canonical form per value.

/// A JS string value stored in a unit (property names, string literals, RegExp patterns).
///
/// Canonical form: a string whose code units are all `<= 0xFF` is always [`UnitString::Latin1`].
/// The constructors enforce it, and the reader rejects a UTF-16 string that fits Latin-1. So
/// equal strings have equal bytes. UTF-16 strings may hold lone surrogates.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnitString {
    /// One byte per code unit (U+0000..=U+00FF).
    Latin1(Box<[u8]>),
    /// UTF-16 code units; at least one is above 0xFF.
    Utf16(Box<[u16]>),
}

impl UnitString {
    /// The string for UTF-8 text.
    pub fn new(s: &str) -> UnitString {
        if s.chars().all(|c| u32::from(c) <= 0xFF) {
            UnitString::Latin1(s.chars().map(|c| c as u8).collect())
        } else {
            UnitString::Utf16(s.encode_utf16().collect())
        }
    }

    /// The string for UTF-16 code units (lone surrogates allowed).
    pub fn from_utf16(units: &[u16]) -> UnitString {
        if units.iter().all(|&u| u <= 0xFF) {
            UnitString::Latin1(units.iter().map(|&u| u as u8).collect())
        } else {
            UnitString::Utf16(units.into())
        }
    }

    /// The string for Latin-1 bytes.
    pub fn from_latin1(bytes: &[u8]) -> UnitString {
        UnitString::Latin1(bytes.into())
    }

    /// Length in UTF-16 code units (JS `length`).
    pub fn len(&self) -> usize {
        match self {
            Self::Latin1(b) => b.len(),
            Self::Utf16(u) => u.len(),
        }
    }

    /// True for the empty string.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Code unit `i`.
    pub fn code_unit(&self, i: usize) -> u16 {
        match self {
            Self::Latin1(b) => u16::from(b[i]),
            Self::Utf16(u) => u[i],
        }
    }

    /// The code units, as a new vector.
    pub fn to_utf16(&self) -> Vec<u16> {
        (0..self.len()).map(|i| self.code_unit(i)).collect()
    }

    /// True if the representation is the canonical one.
    pub fn is_canonical(&self) -> bool {
        match self {
            Self::Latin1(_) => true,
            Self::Utf16(u) => u.iter().any(|&c| c > 0xFF),
        }
    }

    /// Stable quoted form for the disassembly: printable ASCII as is, `"` and `\` escaped,
    /// everything else as `\xHH` (Latin-1) or `\uHHHH` (UTF-16 code unit).
    pub fn quoted(&self) -> String {
        let mut s = String::with_capacity(self.len() + 2);
        s.push('"');
        for i in 0..self.len() {
            let u = self.code_unit(i);
            match u {
                0x22 => s.push_str("\\\""),
                0x5c => s.push_str("\\\\"),
                0x20..=0x7e => s.push(u as u8 as char),
                0..=0xff => s.push_str(&format!("\\x{u:02x}")),
                _ => s.push_str(&format!("\\u{u:04x}")),
            }
        }
        s.push('"');
        s
    }
}

impl From<&str> for UnitString {
    fn from(s: &str) -> UnitString {
        UnitString::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_form() {
        assert!(matches!(UnitString::new("abc"), UnitString::Latin1(_)));
        assert!(matches!(
            UnitString::new("caf\u{e9}"),
            UnitString::Latin1(_)
        ));
        assert!(matches!(UnitString::new("\u{20ac}"), UnitString::Utf16(_)));
        assert_eq!(UnitString::from_utf16(&[0x61, 0x62]), UnitString::new("ab"));
        let lone = UnitString::from_utf16(&[0xD800, 0x61]);
        assert!(matches!(lone, UnitString::Utf16(_)));
        assert_eq!(lone.len(), 2);
        assert!(lone.is_canonical());
        assert!(!UnitString::Utf16(vec![0x61].into()).is_canonical());
        // Astral characters are two code units.
        assert_eq!(UnitString::new("\u{1F600}").len(), 2);
    }

    #[test]
    fn quoting_is_stable() {
        assert_eq!(UnitString::new("a\"b\\c").quoted(), "\"a\\\"b\\\\c\"");
        assert_eq!(UnitString::new("\n\u{e9}").quoted(), "\"\\x0a\\xe9\"");
        assert_eq!(UnitString::from_utf16(&[0xD800]).quoted(), "\"\\ud800\"");
    }
}
