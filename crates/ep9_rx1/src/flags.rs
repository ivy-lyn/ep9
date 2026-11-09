// Copyright 2021 the V8 project authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// Ported for ep9 from V8 src/regexp/regexp-flags.h at 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0.
// The V8 licence text is in THIRD-PARTY-LICENSE (entry "V8 (BSD-3-Clause)").

//! The RegExp flags (V8 `regexp-flags.h`).
//!
//! The bit order is ep9's, not V8's: d = bit 0 ... y = bit 7, the order of the bytecode's
//! `CreateRegExpLiteral` flags operand and of `ep9_regexp::Flags`, so the VM can pass the same
//! byte to RX0 and RX1. V8's `l` (linear) flag is not a JavaScript flag and is not ported.

/// The flags of a RegExp (`dgimsuvy`), one bit each.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Flags(u8);

impl Flags {
    /// `d`: the result has match indices.
    pub const HAS_INDICES: Flags = Flags(1 << 0);
    /// `g`: global.
    pub const GLOBAL: Flags = Flags(1 << 1);
    /// `i`: ignore case.
    pub const IGNORE_CASE: Flags = Flags(1 << 2);
    /// `m`: `^` and `$` also match at line ends.
    pub const MULTILINE: Flags = Flags(1 << 3);
    /// `s`: `.` also matches line ends.
    pub const DOT_ALL: Flags = Flags(1 << 4);
    /// `u`: Unicode mode.
    pub const UNICODE: Flags = Flags(1 << 5);
    /// `v`: Unicode sets mode (also Unicode mode).
    pub const UNICODE_SETS: Flags = Flags(1 << 6);
    /// `y`: sticky.
    pub const STICKY: Flags = Flags(1 << 7);

    /// The flag letters in bit order.
    const LETTERS: [u8; 8] = *b"dgimsuvy";

    /// No flag.
    pub const fn empty() -> Flags {
        Flags(0)
    }

    /// The flags of `bits` (bit order as the constants above).
    pub const fn from_bits(bits: u8) -> Flags {
        Flags(bits)
    }

    /// The bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// True if every flag of `other` is set.
    pub const fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    /// Both sets of flags.
    pub const fn union(self, other: Flags) -> Flags {
        Flags(self.0 | other.0)
    }

    /// `self` with the flags of `other` set (`on`) or cleared (V8 `Flags::set`; the modifiers
    /// of a group, `(?i-m:...)`).
    pub const fn with(self, other: Flags, on: bool) -> Flags {
        if on {
            Flags(self.0 | other.0)
        } else {
            Flags(self.0 & !other.0)
        }
    }

    /// The flag of one letter (V8 `TryFlagFromChar`, without `l`).
    pub fn from_letter(c: u32) -> Option<Flags> {
        let at = Self::LETTERS.iter().position(|&l| u32::from(l) == c)?;
        Some(Flags(1 << at))
    }

    /// The flags of a flags string (RegExpInitialize, ECMA-262 22.2.3.3 step 5): `None` if a
    /// code unit is not one of `dgimsuvy` or a flag comes twice. `u` with `v` is also `None`
    /// (step 11: a SyntaxError of the flags, before the pattern is parsed).
    pub fn parse(units: &[u16]) -> Option<Flags> {
        let mut bits = Flags::empty();
        for &c in units {
            let f = Self::from_letter(u32::from(c))?;
            if bits.contains(f) {
                return None;
            }
            bits = bits.union(f);
        }
        if bits.contains(Flags::UNICODE.union(Flags::UNICODE_SETS)) {
            return None;
        }
        Some(bits)
    }

    /// The flags string in the canonical order `dgimsuvy`.
    pub fn to_letters(self) -> String {
        let mut s = String::with_capacity(8);
        for (i, &l) in Self::LETTERS.iter().enumerate() {
            if self.0 & (1 << i) != 0 {
                s.push(char::from(l));
            }
        }
        s
    }

    /// `i`.
    pub const fn ignore_case(self) -> bool {
        self.contains(Flags::IGNORE_CASE)
    }

    /// `m`.
    pub const fn multiline(self) -> bool {
        self.contains(Flags::MULTILINE)
    }

    /// `s`.
    pub const fn dot_all(self) -> bool {
        self.contains(Flags::DOT_ALL)
    }

    /// `v`.
    pub const fn unicode_sets(self) -> bool {
        self.contains(Flags::UNICODE_SETS)
    }

    /// `u` or `v` (V8 `IsEitherUnicode`; the spec's UnicodeMode).
    pub const fn either_unicode(self) -> bool {
        self.contains(Flags::UNICODE) || self.contains(Flags::UNICODE_SETS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn parse_takes_each_flag_once_and_refuses_u_with_v() {
        assert_eq!(Flags::parse(&units("")), Some(Flags::empty()));
        let all = Flags::parse(&units("ydsmigu")).unwrap();
        assert_eq!(all.to_letters(), "dgimsuy");
        assert_eq!(Flags::parse(&units("gg")), None);
        assert_eq!(Flags::parse(&units("l")), None);
        assert_eq!(Flags::parse(&units("uv")), None);
        assert_eq!(Flags::parse(&units("v")).unwrap(), Flags::UNICODE_SETS);
    }

    #[test]
    fn with_sets_and_clears() {
        let f = Flags::IGNORE_CASE.union(Flags::MULTILINE);
        assert_eq!(f.with(Flags::IGNORE_CASE, false), Flags::MULTILINE);
        assert_eq!(f.with(Flags::DOT_ALL, true).to_letters(), "ims");
    }
}
