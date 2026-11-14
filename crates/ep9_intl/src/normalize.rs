//! Unicode normalization for `String.prototype.normalize` (ECMA-262 §22.1.3.15; block 6 member
//! 6.48 in `ep9_builtins` string.rs). The ICU4X call lives here, not in `ep9_builtins`: only
//! `ep9_intl` depends on `icu_normalizer` (dep-guards checks 24 and 25; INTL-W4-NORMALIZE).
//!
//! The four forms come from the ICU4X 2.4 normalizers with compiled data (Unicode 18).
//!
//! ```text
//! code units ──> runs of well-formed UTF-16, split at each lone surrogate
//!   run ──> icu_normalizer normalize_utf16 (Cow: borrowed when the run is normal already)
//!   lone surrogate ──> copied as it is
//! all runs borrowed ──> Cow::Borrowed(the input): the caller returns S itself, no new string
//! ```
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

    /// The ICU4X normalization of one well-formed run (no lone surrogate in it).
    fn run<'a>(self, run: &'a [u16]) -> Cow<'a, [u16]> {
        match self {
            Form::Nfc => NFC.normalize_utf16(run),
            Form::Nfd => NFD.normalize_utf16(run),
            Form::Nfkc => NFKC.normalize_utf16(run),
            Form::Nfkd => NFKD.normalize_utf16(run),
        }
    }
}

/// Step 6 of String.prototype.normalize: the code units of `s` normalized into `form`.
/// `Cow::Borrowed(s)` when `s` is in that form already (then the caller needs no new string);
/// lone surrogates pass through as they are (see the module doc).
pub fn normalize(s: &[u16], form: Form) -> Cow<'_, [u16]> {
    // `out` stays `None` while every run so far is normal already: the result up to here is
    // then `s[..start]` itself.
    let mut out: Option<Vec<u16>> = None;
    let push_run = |out: &mut Option<Vec<u16>>, start: usize, end: usize| {
        let run = form.run(&s[start..end]);
        match (out.as_mut(), run) {
            (None, Cow::Borrowed(_)) => {}
            (None, Cow::Owned(run)) => {
                let mut v = Vec::with_capacity(s.len() - (end - start) + run.len());
                v.extend_from_slice(&s[..start]);
                v.extend_from_slice(&run);
                *out = Some(v);
            }
            (Some(v), run) => v.extend_from_slice(&run),
        }
    };
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
            push_run(&mut out, start, i);
            if let Some(v) = out.as_mut() {
                v.push(u);
            }
            start = i + 1;
        }
        i += 1;
    }
    push_run(&mut out, start, s.len());
    match out {
        None => Cow::Borrowed(s),
        Some(v) => Cow::Owned(v),
    }
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
                matches!(normalize(&units, form), Cow::Borrowed(_)),
                "{s:?} {form:?}"
            );
        }
        // Lone surrogates around normal runs: still borrowed.
        let units = [0xDC00, 0x61, 0xD800, 0x62, 0xD83D, 0xDE00, 0xD800];
        assert!(matches!(normalize(&units, Form::Nfc), Cow::Borrowed(_)));
    }

    #[test]
    fn the_four_forms_of_the_test262_string() {
        // test262 built-ins/String/prototype/normalize/return-normalized-string.js.
        let s = u("\u{1E9B}\u{323}");
        assert_eq!(normalize(&s, Form::Nfc).as_ref(), u("\u{1E9B}\u{323}"));
        assert_eq!(
            normalize(&s, Form::Nfd).as_ref(),
            u("\u{17F}\u{323}\u{307}")
        );
        assert_eq!(normalize(&s, Form::Nfkc).as_ref(), u("\u{1E69}"));
        assert_eq!(normalize(&s, Form::Nfkd).as_ref(), u("s\u{323}\u{307}"));
    }

    #[test]
    fn lone_surrogates_pass_through_and_split_the_runs() {
        // "a" U+0301 <lone high> "e" U+0301 <lone low> U+0301: the mark after a lone surrogate
        // does not compose with the "e" before it.
        let s = [0x61, 0x301, 0xD800, 0x65, 0x301, 0xDC00, 0x301];
        assert_eq!(
            normalize(&s, Form::Nfc).as_ref(),
            [0xE1, 0xD800, 0xE9, 0xDC00, 0x301]
        );
        let nfc = [0xE1, 0xD800, 0xE9, 0xDC00];
        assert_eq!(
            normalize(&nfc, Form::Nfd).as_ref(),
            [0x61, 0x301, 0xD800, 0x65, 0x301, 0xDC00]
        );
        // A pair stays one code point: U+1D15E MUSICAL SYMBOL HALF NOTE decomposes (NFC and
        // NFD both, a composition exclusion) to U+1D157 U+1D165.
        let pair = [0xD834, 0xDD5E];
        assert_eq!(
            normalize(&pair, Form::Nfc).as_ref(),
            [0xD834, 0xDD57, 0xD834, 0xDD65]
        );
        // A lone surrogate first and last, with a change in between.
        let s = [0xDFFF, 0x41, 0x30A, 0xDBFF];
        assert_eq!(normalize(&s, Form::Nfkc).as_ref(), [0xDFFF, 0xC5, 0xDBFF]);
    }
}
