//! Unicode normalization for `String.prototype.normalize` (ECMA-262 §22.1.3.15; block 6 member
//! 6.48 in `ep9_builtins` string.rs). The ICU4X call lives here, not in `ep9_builtins`: only
//! `ep9_intl` depends on `icu_normalizer` (dep-guards checks 24 and 25; INTL-W4-NORMALIZE).
//!
//! The four forms come from the ICU4X 2.4 normalizers with compiled data (Unicode 18).
//!
//! ```text
//! code units ──> runs of well-formed UTF-16, split at each lone surrogate
//!   run ──> split_normalized_utf16: (head, tail)
//!     head ──> copied as it is (normal already)
//!     tail ──> normalize_iter, one char at a time
//!   lone surrogate ──> copied as it is
//! all tails empty ──> Cow::Borrowed(the input): the caller returns S itself, no new string
//! ```
//!
//! The result buffer never grows past `max` code units (W4 Codex r1 F2): one more unit gives
//! `Err(TooLong)` before the buffer grows. One char can be 18 code units (U+FDFA in NFKC), so
//! a full ICU4X result buffer first (`normalize_utf16`) and then a copy gave two buffers of
//! 1 GB or more outside the managed heap for a 60 MB input, and only then the RangeError.
//!
//! ICU4X maps a lone surrogate to U+FFFD before it normalizes; ECMA-262 normalizes the code
//! points of S (StringToCodePoints, §6.1.4), so a lone surrogate is a code point of its own and
//! stays in the result (V8 and SpiderMonkey keep it too). The split is exact: a surrogate code
//! point (General_Category Cs) has canonical combining class 0, no decomposition and no
//! composition, so the canonical reorder stops at it and it blocks each composition across
//! it (UAX #15 §1.3, D115). The normalization of S is then the normalization of each run with
//! the lone surrogates between them.

use std::borrow::Cow;

use icu_normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};

/// The normalizers of the four forms (compiled data; `const`, so no data load at run time).
const NFC: ComposingNormalizerBorrowed<'static> = ComposingNormalizerBorrowed::new_nfc();
const NFKC: ComposingNormalizerBorrowed<'static> = ComposingNormalizerBorrowed::new_nfkc();
const NFD: DecomposingNormalizerBorrowed<'static> = DecomposingNormalizerBorrowed::new_nfd();
const NFKD: DecomposingNormalizerBorrowed<'static> = DecomposingNormalizerBorrowed::new_nfkd();

/// A normalization form of String.prototype.normalize step 5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// "NFC": canonical decomposition, then canonical composition.
    Nfc,
    /// "NFD": canonical decomposition.
    Nfd,
    /// "NFKC": compatibility decomposition, then canonical composition.
    Nfkc,
    /// "NFKD": compatibility decomposition.
    Nfkd,
}

impl Form {
    /// The form named by the code units `f` (step 5: one of "NFC", "NFD", "NFKC" or "NFKD",
    /// with the same case), or `None` for another string (the caller throws a RangeError).
    pub fn from_units(f: &[u16]) -> Option<Self> {
        const NAMES: [(&[u8], Form); 4] = [
            (b"NFC", Form::Nfc),
            (b"NFD", Form::Nfd),
            (b"NFKC", Form::Nfkc),
            (b"NFKD", Form::Nfkd),
        ];
        NAMES.iter().find_map(|&(name, form)| {
            (name.len() == f.len() && name.iter().zip(f).all(|(&b, &u)| u16::from(b) == u))
                .then_some(form)
        })
    }

    /// The ICU4X split of one well-formed run (no lone surrogate in it): the longest head
    /// that is normal already, and the tail. The result of the run is the head and then the
    /// normalization of the tail.
    fn split(self, run: &[u16]) -> (&[u16], &[u16]) {
        match self {
            Form::Nfc => NFC.split_normalized_utf16(run),
            Form::Nfd => NFD.split_normalized_utf16(run),
            Form::Nfkc => NFKC.split_normalized_utf16(run),
            Form::Nfkd => NFKD.split_normalized_utf16(run),
        }
    }

    /// Appends the normalization of `tail` (well-formed UTF-16, the tail of [`Form::split`])
    /// to `out`, one char at a time; `Err(TooLong)` at the first char past the limit.
    fn write(self, tail: &[u16], out: &mut Bounded) -> Result<(), TooLong> {
        // The run has no lone surrogate, so no decode error (U+FFFD is never used).
        let chars = char::decode_utf16(tail.iter().copied()).map(|c| c.unwrap_or('\u{FFFD}'));
        match self {
            Form::Nfc => NFC.normalize_iter(chars).try_for_each(|c| out.push_char(c)),
            Form::Nfd => NFD.normalize_iter(chars).try_for_each(|c| out.push_char(c)),
            Form::Nfkc => NFKC
                .normalize_iter(chars)
                .try_for_each(|c| out.push_char(c)),
            Form::Nfkd => NFKD
                .normalize_iter(chars)
                .try_for_each(|c| out.push_char(c)),
        }
    }
}

/// The normalized string would be longer than the `max` of [`normalize`] (the caller throws
/// the RangeError of a too long string).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLong;

/// A result buffer that never holds, or reserves, more than `max` code units.
struct Bounded {
    v: Vec<u16>,
    max: usize,
}

impl Bounded {
    fn push_slice(&mut self, more: &[u16]) -> Result<(), TooLong> {
        if more.len() > self.max - self.v.len() {
            return Err(TooLong);
        }
        let need = self.v.len() + more.len();
        if need > self.v.capacity() {
            // Double as a Vec does, but never reserve past `max`.
            let want = need.max(self.v.capacity().saturating_mul(2)).min(self.max);
            self.v.reserve_exact(want - self.v.len());
        }
        self.v.extend_from_slice(more);
        Ok(())
    }

    fn push_char(&mut self, c: char) -> Result<(), TooLong> {
        let mut b = [0u16; 2];
        self.push_slice(c.encode_utf16(&mut b))
    }
}

/// Appends the normalization of the run `s[start..end]` (`out` is `None` while every run so
/// far is normal already: the result up to `start` is then `s[..start]` itself).
fn push_run(
    s: &[u16],
    form: Form,
    max: usize,
    out: &mut Option<Bounded>,
    start: usize,
    end: usize,
) -> Result<(), TooLong> {
    let (head, tail) = form.split(&s[start..end]);
    let o = match out {
        Some(o) => {
            o.push_slice(head)?;
            o
        }
        None if tail.is_empty() => return Ok(()),
        None => {
            let mut o = Bounded {
                v: Vec::with_capacity(s.len().min(max)),
                max,
            };
            o.push_slice(&s[..start + head.len()])?;
            out.insert(o)
        }
    };
    form.write(tail, o)
}

/// Step 6 of String.prototype.normalize: the code units of `s` normalized into `form`.
/// `Cow::Borrowed(s)` when `s` is in that form already (then the caller needs no new string);
/// lone surrogates pass through as they are (see the module doc). `Err(TooLong)` when the
/// result is longer than `max` code units: the buffer stops there (it never holds more).
pub fn normalize(s: &[u16], form: Form, max: usize) -> Result<Cow<'_, [u16]>, TooLong> {
    let mut out: Option<Bounded> = None;
    let mut start = 0;
    let mut i = 0;
    while i < s.len() {
        let u = s[i];
        if (0xD800..=0xDBFF).contains(&u) && matches!(s.get(i + 1), Some(0xDC00..=0xDFFF)) {
            // A surrogate pair: one code point of the run.
            i += 2;
            continue;
        }
        if (0xD800..=0xDFFF).contains(&u) {
            // A lone surrogate: end the run before it and copy it.
            push_run(s, form, max, &mut out, start, i)?;
            if let Some(o) = out.as_mut() {
                o.push_slice(&[u])?;
            }
            start = i + 1;
        }
        i += 1;
    }
    push_run(s, form, max, &mut out, start, s.len())?;
    Ok(match out {
        None => Cow::Borrowed(s),
        Some(o) => Cow::Owned(o.v),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn the_form_names_are_exact() {
        assert_eq!(Form::from_units(&u("NFC")), Some(Form::Nfc));
        assert_eq!(Form::from_units(&u("NFD")), Some(Form::Nfd));
        assert_eq!(Form::from_units(&u("NFKC")), Some(Form::Nfkc));
        assert_eq!(Form::from_units(&u("NFKD")), Some(Form::Nfkd));
        for bad in ["nfc", "NFC ", "NF", "", "NFKCabc", "NFE", "\u{FF2E}FC"] {
            assert_eq!(Form::from_units(&u(bad)), None, "{bad:?}");
        }
    }

    #[test]
    fn a_normal_string_is_borrowed() {
        for (s, form) in [
            ("abc", Form::Nfc),
            ("\u{E1}", Form::Nfc),
            ("a\u{301}", Form::Nfd),
            ("", Form::Nfkd),
        ] {
            let units = u(s);
            assert!(
                matches!(normalize(&units, form, usize::MAX), Ok(Cow::Borrowed(_))),
                "{s:?} {form:?}"
            );
        }
        // Lone surrogates around normal runs: still borrowed.
        let units = [0xDC00, 0x61, 0xD800, 0x62, 0xD83D, 0xDE00, 0xD800];
        assert!(matches!(
            normalize(&units, Form::Nfc, usize::MAX),
            Ok(Cow::Borrowed(_))
        ));
    }

    #[test]
    fn the_four_forms_of_the_test262_string() {
        // test262 built-ins/String/prototype/normalize/return-normalized-string.js.
        let s = u("\u{1E9B}\u{323}");
        assert_eq!(
            normalize(&s, Form::Nfc, usize::MAX).unwrap().as_ref(),
            u("\u{1E9B}\u{323}")
        );
        assert_eq!(
            normalize(&s, Form::Nfd, usize::MAX).unwrap().as_ref(),
            u("\u{17F}\u{323}\u{307}")
        );
        assert_eq!(
            normalize(&s, Form::Nfkc, usize::MAX).unwrap().as_ref(),
            u("\u{1E69}")
        );
        assert_eq!(
            normalize(&s, Form::Nfkd, usize::MAX).unwrap().as_ref(),
            u("s\u{323}\u{307}")
        );
    }

    #[test]
    fn lone_surrogates_pass_through_and_split_the_runs() {
        // "a" U+0301 <lone high> "e" U+0301 <lone low> U+0301: the mark after a lone surrogate
        // does not compose with the "e" before it.
        let s = [0x61, 0x301, 0xD800, 0x65, 0x301, 0xDC00, 0x301];
        assert_eq!(
            normalize(&s, Form::Nfc, usize::MAX).unwrap().as_ref(),
            [0xE1, 0xD800, 0xE9, 0xDC00, 0x301]
        );
        let nfc = [0xE1, 0xD800, 0xE9, 0xDC00];
        assert_eq!(
            normalize(&nfc, Form::Nfd, usize::MAX).unwrap().as_ref(),
            [0x61, 0x301, 0xD800, 0x65, 0x301, 0xDC00]
        );
        // A pair stays one code point: U+1D15E MUSICAL SYMBOL HALF NOTE decomposes (NFC and
        // NFD both, a composition exclusion) to U+1D157 U+1D165.
        let pair = [0xD834, 0xDD5E];
        assert_eq!(
            normalize(&pair, Form::Nfc, usize::MAX).unwrap().as_ref(),
            [0xD834, 0xDD57, 0xD834, 0xDD65]
        );
        // A lone surrogate first and last, with a change in between.
        let s = [0xDFFF, 0x41, 0x30A, 0xDBFF];
        assert_eq!(
            normalize(&s, Form::Nfkc, usize::MAX).unwrap().as_ref(),
            [0xDFFF, 0xC5, 0xDBFF]
        );
    }

    /// W4 Codex r1 F2: the result stops at `max` code units. U+FDFA is 18 units in NFKC and
    /// NFKD; it is NFC and NFD already, so those stay borrowed whatever the max.
    #[test]
    fn the_result_stops_at_the_limit() {
        let s = u(&"\u{FDFA}".repeat(1000));
        for form in [Form::Nfkc, Form::Nfkd] {
            let full = normalize(&s, form, usize::MAX).unwrap();
            assert_eq!(full.len(), 18_000, "{form:?}");
            assert_eq!(normalize(&s, form, 18_000).unwrap(), full, "{form:?}");
            assert_eq!(normalize(&s, form, 17_999), Err(TooLong), "{form:?}");
            assert_eq!(normalize(&s, form, 0), Err(TooLong), "{form:?}");
        }
        for form in [Form::Nfc, Form::Nfd] {
            assert!(
                matches!(normalize(&s, form, 1000), Ok(Cow::Borrowed(_))),
                "{form:?}"
            );
        }
        // A head that is normal already counts too: "a" x 10 then U+FDFA.
        let s = u("aaaaaaaaaa\u{FDFA}");
        assert_eq!(normalize(&s, Form::Nfkc, 28).map(|r| r.len()), Ok(28));
        assert_eq!(normalize(&s, Form::Nfkc, 27), Err(TooLong));
        // A lone surrogate after the change counts too.
        let s = [0xFDFA, 0xD800];
        assert_eq!(normalize(&s, Form::Nfkc, 19).map(|r| r.len()), Ok(19));
        assert_eq!(normalize(&s, Form::Nfkc, 18), Err(TooLong));
    }

    /// The tail goes through `normalize_iter`: it gives the same units as ICU4X's
    /// `normalize_utf16` for each scalar value of planes 0-2 (every one with a decomposition
    /// or a nonzero combining class is there) alone, after "a", and before U+0301 U+0323.
    #[test]
    fn the_iterator_path_matches_normalize_utf16() {
        let mut units = Vec::new();
        for c in (0..=0x2FFFFu32).filter_map(char::from_u32) {
            for s in [
                format!("{c}"),
                format!("a{c}"),
                format!("{c}\u{301}\u{323}"),
            ] {
                units.clear();
                units.extend(s.encode_utf16());
                for form in [Form::Nfc, Form::Nfd, Form::Nfkc, Form::Nfkd] {
                    let want = match form {
                        Form::Nfc => NFC.normalize_utf16(&units),
                        Form::Nfd => NFD.normalize_utf16(&units),
                        Form::Nfkc => NFKC.normalize_utf16(&units),
                        Form::Nfkd => NFKD.normalize_utf16(&units),
                    };
                    let got = normalize(&units, form, usize::MAX).unwrap();
                    assert_eq!(got, want, "{:x} {form:?}", u32::from(c));
                }
            }
        }
    }
}
