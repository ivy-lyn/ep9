// Copyright 2019 the V8 project authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// Ported for ep9 from V8 src/regexp/special-case.h (class CaseFolding) and
// src/regexp/regexp-compiler.cc (TextNode::GetCaseIndependentLetters) at
// 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0.
// The V8 licence text is in THIRD-PARTY-LICENSE (entry "V8 (BSD-3-Clause)").

//! Case equivalence for `i` (V8 `CaseFolding`, `special-case.h`).
//!
//! ```text
//! V8 (ICU)                                         RX1 (unicode_sets.rs, gen-sets.py)
//! UnicodeSet::closeOver(USET_SIMPLE_CASE_          CASE_ORBIT: the Simple_Case_Folding classes
//!   INSENSITIVE)                                     as cycles
//! u_foldCase(c, U_FOLD_CASE_DEFAULT)               SIMPLE_FOLDING
//! IgnoreSet, SupplementaryEquivalentsSet           IGNORE_SET, SUPPLEMENTARY_EQUIVALENTS
//!   (gen-regexp-special-case.cc, at build time)      (gen-sets.py ports that generator)
//! ```
//!
//! Without `u` (ECMA-262 Canonicalize with rer.[[Unicode]] false), two code units match when
//! their Canonicalize values are equal; with `u` or `v`, when their Simple_Case_Folding values
//! are equal. The generator checks V8's claim that [`close_over`] with [`Mode::NonUnicode`]
//! gives the Canonicalize classes, for every BMP code unit.

use crate::charset::{self, CharacterRange};
use crate::flags::Flags;
use crate::unicode_sets::{CASE_ORBIT, IGNORE_SET, SIMPLE_FOLDING, SUPPLEMENTARY_EQUIVALENTS};

/// V8 `CaseFolding::Mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `i` without `u` or `v`: UTF-16 code units, ECMA-262 Canonicalize.
    NonUnicode,
    /// `u` or `v` with `i`: code points, Simple_Case_Folding.
    Unicode,
}

fn in_table(table: &[(u32, u32)], c: u32) -> bool {
    let i = table.partition_point(|&(_, to)| to < c);
    table.get(i).is_some_and(|&(from, _)| from <= c)
}

/// ICU `u_foldCase(c, U_FOLD_CASE_DEFAULT)`: the Simple_Case_Folding of `c`.
#[must_use]
pub fn simple_fold(c: u32) -> u32 {
    match SIMPLE_FOLDING.binary_search_by_key(&c, |&(from, _)| from) {
        Ok(i) => SIMPLE_FOLDING[i].1,
        Err(_) => c,
    }
}

/// V8 `CaseFolding::EquivalenceKey`: equal keys are matching characters under `i` (the key
/// is not always the spec's Canonicalize value). Without Unicode, `c` is a code unit.
#[must_use]
pub fn equivalence_key(c: u32, mode: Mode) -> u32 {
    debug_assert!(
        c <= if mode == Mode::NonUnicode {
            0xffff
        } else {
            0x10_ffff
        }
    );
    if mode == Mode::NonUnicode && in_table(IGNORE_SET, c) {
        return c;
    }
    simple_fold(c)
}

/// V8 `CaseFolding::HasSupplementaryEquivalents`: the Simple_Case_Folding class of the BMP
/// character `c` holds a code point above U+FFFF (none does in Unicode 17.0.0).
#[must_use]
pub fn has_supplementary_equivalents(c: u16) -> bool {
    in_table(SUPPLEMENTARY_EQUIVALENTS, u32::from(c))
}

/// The other members of the Simple_Case_Folding class of `c` (none if its class is `{c}`).
fn for_each_equivalent(c: u32, mut f: impl FnMut(u32)) {
    let Ok(start) = CASE_ORBIT.binary_search_by_key(&c, |&(from, _)| from) else {
        return;
    };
    let mut next = CASE_ORBIT[start].1;
    while next != c {
        f(next);
        let i = CASE_ORBIT
            .binary_search_by_key(&next, |&(from, _)| from)
            .expect("a CASE_ORBIT cycle is closed");
        next = CASE_ORBIT[i].1;
    }
}

/// ICU `closeOver(USET_SIMPLE_CASE_INSENSITIVE)` on a canonical list; canonical after.
fn simple_closure(ranges: &mut Vec<CharacterRange>) {
    let mut added = Vec::new();
    for r in ranges.iter() {
        let start = CASE_ORBIT.partition_point(|&(c, _)| c < r.from());
        for &(c, _) in CASE_ORBIT[start..]
            .iter()
            .take_while(|&&(c, _)| c <= r.to())
        {
            for_each_equivalent(c, |e| added.push(CharacterRange::singleton(e)));
        }
    }
    if !added.is_empty() {
        ranges.extend(added);
        charset::canonicalize(ranges);
    }
}

/// V8 `CaseFolding::CloseOver`: closes a canonical list under case equivalence and keeps its
/// own members; canonical after. With [`Mode::NonUnicode`] the list holds only code units,
/// and the IgnoreSet members neither add nor get equivalents (V8 has three arms for this;
/// they give `set | (closure(set - IgnoreSet) - IgnoreSet - supplementary)`, the one form
/// here).
pub fn close_over(ranges: &mut Vec<CharacterRange>, mode: Mode) {
    debug_assert!(charset::is_canonical(ranges));
    if mode == Mode::Unicode {
        simple_closure(ranges);
        return;
    }
    debug_assert!(ranges.last().is_none_or(|r| r.to() <= 0xffff));
    let ignore: Vec<CharacterRange> = IGNORE_SET
        .iter()
        .map(|&(f, t)| CharacterRange::range(f, t))
        .collect();
    let mut closed = charset::subtract(ranges, &ignore);
    simple_closure(&mut closed);
    let closed = charset::subtract(&closed, &ignore);
    let closed = charset::subtract(&closed, &[CharacterRange::range(0x10000, 0x10_ffff)]);
    ranges.extend(closed);
    charset::canonicalize(ranges);
}

/// V8 `TextBuilder::NeedsDesugaringForIgnoreCase`: with `u` or `v` and `i`, a supplementary
/// character with case equivalents, or a BMP character with supplementary equivalents, is a
/// one-character class in the tree, not text.
#[must_use]
pub(crate) fn needs_desugaring_for_ignore_case(c: u32, flags: Flags) -> bool {
    if !(flags.either_unicode() && flags.ignore_case()) {
        return false;
    }
    if c <= 0xffff {
        return has_supplementary_equivalents(c as u16);
    }
    CASE_ORBIT
        .binary_search_by_key(&c, |&(from, _)| from)
        .is_ok()
}

/// V8 `TextNode::GetCaseIndependentLetters`: the code units that match the code unit `c`
/// under `i`, `c` first for ASCII and then in code unit order (the ICU set order); with
/// `one_byte_subject`, only the Latin-1 ones. In Unicode mode the supplementary equivalents
/// are not here (the parser desugars such characters into classes).
#[must_use]
pub fn case_independent_letters(c: u16, unicode: bool, one_byte_subject: bool) -> Vec<u16> {
    const MAX_ASCII: u16 = 0x7f;
    if !unicode && c <= MAX_ASCII {
        let upper = c & !0x20;
        if (u16::from(b'A')..=u16::from(b'Z')).contains(&upper) {
            return vec![upper, upper | 0x20];
        }
        return vec![c];
    }
    let mut set = vec![CharacterRange::singleton(u32::from(c))];
    close_over(
        &mut set,
        if unicode {
            Mode::Unicode
        } else {
            Mode::NonUnicode
        },
    );
    let mut out = Vec::new();
    for r in set {
        for cu in r.from()..=r.to() {
            if one_byte_subject && cu > 0xff {
                continue;
            }
            if let Ok(u) = u16::try_from(cu) {
                out.push(u);
            }
        }
    }
    out
}
