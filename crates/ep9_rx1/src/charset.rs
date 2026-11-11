// Copyright 2019 the V8 project authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// Ported for ep9 from V8 src/regexp/regexp-ast.h (class CharacterRange),
// src/regexp/regexp-compiler-tonode.cc (the CharacterRange functions) and
// src/regexp/regexp-compiler.h (compiler_constants) at 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0.
// The V8 licence text is in THIRD-PARTY-LICENSE (entry "V8 (BSD-3-Clause)").

//! Code point ranges and range lists (V8 `CharacterRange`).
//!
//! ```text
//! V8 (ZoneList<CharacterRange>*, in place)   RX1 (Vec<CharacterRange>)
//! Canonicalize(list)                          canonicalize(&mut v): sort, merge touching ranges
//! Negate(src, dst), Intersect, Subtract       negate(&s), intersect(&a, &b), subtract(&a, &b)
//! AddClassEscape(set, list, closure?)         add_class_escape(set, &mut v, closure?)
//! AddCaseEquivalents (/i)                     add_case_equivalents(&mut v, one_byte)
//! AddUnicodeCaseEquivalents (/ui, /vi)        add_unicode_case_equivalents(&mut v)
//! ```
//!
//! A list is canonical when its ranges are sorted and no two overlap or touch (V8
//! `IsCanonical`). [`negate`], [`intersect`] and [`subtract`] take and give canonical lists.
//!
//! One change from V8, by name: V8 `Negate` adds the last range only `if (from <
//! kMaxCodePoint)`, so the complement of a list that ends at U+10FFFE drops U+10FFFF (node
//! v26.13.0: `/[^\0-\u{10FFFE}]/u.test("\u{10FFFF}")` is false). ECMA-262
//! CharacterComplement keeps it; RX1 adds the range when `from <= MAX_CODE_POINT`.

use crate::ast::StandardSet;
use crate::special_case::{self, Mode};

/// The largest code point (V8 `kMaxCodePoint`).
pub const MAX_CODE_POINT: u32 = 0x10ffff;
/// The largest UTF-16 code unit (V8 `kMaxUtf16CodeUnit`).
pub(crate) const MAX_UTF16_CODE_UNIT: u32 = 0xffff;
/// The largest Latin-1 code unit (V8 `String::kMaxOneByteCharCode`).
pub(crate) const MAX_ONE_BYTE_CHAR_CODE: u32 = 0xff;
/// V8 `kLeadSurrogateStart`.
pub(crate) const LEAD_SURROGATE_START: u32 = 0xd800;
/// V8 `kTrailSurrogateEnd`.
pub(crate) const TRAIL_SURROGATE_END: u32 = 0xdfff;

/// The code points `from..=to` (V8 `CharacterRange`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CharacterRange {
    from: u32,
    to: u32,
}

impl CharacterRange {
    /// `from..=to` (V8 `Range`). `from <= to <= MAX_CODE_POINT`.
    #[must_use]
    pub const fn range(from: u32, to: u32) -> CharacterRange {
        debug_assert!(from <= to && to <= MAX_CODE_POINT);
        CharacterRange { from, to }
    }

    /// The one code point `c` (V8 `Singleton`).
    #[must_use]
    pub const fn singleton(c: u32) -> CharacterRange {
        CharacterRange::range(c, c)
    }

    /// Every code point (V8 `Everything`).
    #[must_use]
    pub const fn everything() -> CharacterRange {
        CharacterRange::range(0, MAX_CODE_POINT)
    }

    /// The first code point.
    #[must_use]
    pub const fn from(self) -> u32 {
        self.from
    }

    /// The last code point.
    #[must_use]
    pub const fn to(self) -> u32 {
        self.to
    }

    /// V8 `Contains`.
    #[must_use]
    pub const fn contains(self, c: u32) -> bool {
        self.from <= c && c <= self.to
    }

    /// V8 `IsEverything(max)`: the range starts at 0 and reaches `max`.
    #[must_use]
    pub const fn is_everything(self, max: u32) -> bool {
        self.from == 0 && self.to >= max
    }

    /// V8 `IsSingleton`.
    #[must_use]
    pub const fn is_singleton(self) -> bool {
        self.from == self.to
    }
}

/// True if `ranges` is sorted and no two ranges overlap or touch (V8 `IsCanonical`).
#[must_use]
pub fn is_canonical(ranges: &[CharacterRange]) -> bool {
    ranges.windows(2).all(|w| w[0].to + 1 < w[1].from)
}

/// Sorts `ranges` and merges the ranges that overlap or touch (V8 `Canonicalize`; V8 inserts
/// each range into a sorted prefix, which gives the same list).
pub fn canonicalize(ranges: &mut Vec<CharacterRange>) {
    if is_canonical(ranges) {
        return;
    }
    ranges.sort_unstable();
    let mut out: Vec<CharacterRange> = Vec::with_capacity(ranges.len());
    for &r in ranges.iter() {
        match out.last_mut() {
            Some(last) if r.from <= last.to.saturating_add(1) => last.to = last.to.max(r.to),
            _ => out.push(r),
        }
    }
    *ranges = out;
}

/// The complement of a canonical list over `0..=MAX_CODE_POINT` (V8 `Negate`; see the module
/// doc for the U+10FFFF change).
#[must_use]
pub fn negate(ranges: &[CharacterRange]) -> Vec<CharacterRange> {
    debug_assert!(is_canonical(ranges));
    let mut out = Vec::with_capacity(ranges.len() + 1);
    let mut from = 0u32;
    let mut rest = ranges;
    if let Some(first) = ranges.first()
        && first.from == 0
    {
        from = first.to + 1;
        rest = &ranges[1..];
    }
    for r in rest {
        out.push(CharacterRange::range(from, r.from - 1));
        from = r.to + 1;
    }
    if from <= MAX_CODE_POINT {
        out.push(CharacterRange::range(from, MAX_CODE_POINT));
    }
    out
}

/// The code points in both canonical lists (V8 `Intersect`).
#[must_use]
pub fn intersect(lhs: &[CharacterRange], rhs: &[CharacterRange]) -> Vec<CharacterRange> {
    debug_assert!(is_canonical(lhs) && is_canonical(rhs));
    let mut out = Vec::new();
    let (mut l, mut r) = (0, 0);
    while l < lhs.len() && r < rhs.len() {
        if lhs[l].to < rhs[r].from {
            l += 1;
            continue;
        }
        if rhs[r].to < lhs[l].from {
            r += 1;
            continue;
        }
        let from = lhs[l].from.max(rhs[r].from);
        let to = lhs[l].to.min(rhs[r].to);
        out.push(CharacterRange::range(from, to));
        if to == lhs[l].to {
            l += 1;
        } else {
            r += 1;
        }
    }
    debug_assert!(is_canonical(&out));
    out
}

/// True if the two canonical lists share a code point (V8 `Intersects`).
#[must_use]
pub fn intersects(lhs: &[CharacterRange], rhs: &[CharacterRange]) -> bool {
    let (mut l, mut r) = (0, 0);
    while l < lhs.len() && r < rhs.len() {
        if lhs[l].to < rhs[r].from {
            l += 1;
        } else if rhs[r].to < lhs[l].from {
            r += 1;
        } else {
            return true;
        }
    }
    false
}

/// The code points of `src` that are not in `to_remove`, both canonical (V8 `Subtract`).
#[must_use]
pub fn subtract(src: &[CharacterRange], to_remove: &[CharacterRange]) -> Vec<CharacterRange> {
    debug_assert!(is_canonical(src) && is_canonical(to_remove));
    let mut out = Vec::new();
    let Some(&first) = src.first() else {
        return out;
    };
    let (mut s, mut t) = (0, 0);
    let (mut from, mut to) = (first.from, first.to);
    // V8 SafeAdvanceRange: the next source range, or `from` past the last code point.
    let advance = |s: &mut usize, from: &mut u32, to: &mut u32| {
        *s += 1;
        if let Some(r) = src.get(*s) {
            *from = r.from;
            *to = r.to;
        } else {
            *from = MAX_CODE_POINT + 1;
        }
    };
    while s < src.len() && t < to_remove.len() {
        let remove = to_remove[t];
        if remove.to < from {
            // (a) The remove range is before the current range.
            t += 1;
        } else if to < remove.from {
            // (b) The remove range is after the current range: keep all of it.
            out.push(CharacterRange::range(from, to));
            advance(&mut s, &mut from, &mut to);
        } else if from >= remove.from && to <= remove.to {
            // (c) The remove range covers the current range.
            advance(&mut s, &mut from, &mut to);
        } else if from < remove.from && to > remove.to {
            // (d) The remove range splits the current range.
            out.push(CharacterRange::range(from, remove.from - 1));
            from = remove.to + 1;
            t += 1;
        } else if from < remove.from {
            // (e) The remove range cuts the end of the current range.
            to = remove.from - 1;
            out.push(CharacterRange::range(from, to));
            advance(&mut s, &mut from, &mut to);
        } else {
            // (f) The remove range cuts the start of the current range.
            debug_assert!(to > remove.to);
            from = remove.to + 1;
            t += 1;
        }
    }
    if from <= to && from <= MAX_CODE_POINT {
        out.push(CharacterRange::range(from, to));
    }
    s += 1;
    out.extend_from_slice(src.get(s..).unwrap_or(&[]));
    debug_assert!(is_canonical(&out));
    out
}

/// Drops the code points above U+00FF (V8 `ClampToOneByte`).
pub fn clamp_to_one_byte(ranges: &mut Vec<CharacterRange>) {
    debug_assert!(is_canonical(ranges));
    while let Some(last) = ranges.last_mut() {
        if last.from <= MAX_ONE_BYTE_CHAR_CODE {
            last.to = last.to.min(MAX_ONE_BYTE_CHAR_CODE);
            break;
        }
        ranges.pop();
    }
}

// ---- compiler_constants (regexp-compiler.h): half-open pairs, as in V8 ----------------------

/// `\s`: WhiteSpace and LineTerminator (V8 `kSpaceRanges`, `[from, to)` pairs).
const SPACE_RANGES: &[(u32, u32)] = &[
    (0x09, 0x0e),
    (0x20, 0x21),
    (0xa0, 0xa1),
    (0x1680, 0x1681),
    (0x2000, 0x200b),
    (0x2028, 0x202a),
    (0x202f, 0x2030),
    (0x205f, 0x2060),
    (0x3000, 0x3001),
    (0xfeff, 0xff00),
];
/// `\w` (V8 `kWordRanges`).
const WORD_RANGES: &[(u32, u32)] = &[(0x30, 0x3a), (0x41, 0x5b), (0x5f, 0x60), (0x61, 0x7b)];
/// `\d` (V8 `kDigitRanges`).
const DIGIT_RANGES: &[(u32, u32)] = &[(0x30, 0x3a)];
/// The line terminators of `^`, `$` with `m` and of `.` (V8 `kLineTerminatorRanges`).
const LINE_TERMINATOR_RANGES: &[(u32, u32)] = &[(0x0a, 0x0b), (0x0d, 0x0e), (0x2028, 0x202a)];

/// V8 `AddClass`.
fn add_class(elems: &[(u32, u32)], ranges: &mut Vec<CharacterRange>) {
    for &(from, end) in elems {
        ranges.push(CharacterRange::range(from, end - 1));
    }
}

/// V8 `AddClassNegated`.
fn add_class_negated(elems: &[(u32, u32)], ranges: &mut Vec<CharacterRange>) {
    debug_assert_ne!(elems[0].0, 0);
    let mut last = 0;
    for &(from, end) in elems {
        ranges.push(CharacterRange::range(last, from - 1));
        last = end;
    }
    ranges.push(CharacterRange::range(last, MAX_CODE_POINT));
}

/// Appends the code points of a standard set (V8 `AddClassEscape`). With
/// `add_unicode_case_equivalents` (`u` or `v` with `i`), `\w` and `\W` take the case closure
/// of the word characters before the complement (ECMA-262 WordCharacters, 22.2.2.9.4).
pub fn add_class_escape(
    set: StandardSet,
    ranges: &mut Vec<CharacterRange>,
    add_unicode_case_equivalents: bool,
) {
    if add_unicode_case_equivalents && matches!(set, StandardSet::Word | StandardSet::NotWord) {
        let mut word = Vec::new();
        add_class(WORD_RANGES, &mut word);
        add_unicode_case_equivalents_to(&mut word);
        if set == StandardSet::NotWord {
            word = negate(&word);
        }
        ranges.extend(word);
        return;
    }
    match set {
        StandardSet::Whitespace => add_class(SPACE_RANGES, ranges),
        StandardSet::NotWhitespace => add_class_negated(SPACE_RANGES, ranges),
        StandardSet::Word => add_class(WORD_RANGES, ranges),
        StandardSet::NotWord => add_class_negated(WORD_RANGES, ranges),
        StandardSet::Digit => add_class(DIGIT_RANGES, ranges),
        StandardSet::NotDigit => add_class_negated(DIGIT_RANGES, ranges),
        StandardSet::NotLineTerminator => add_class_negated(LINE_TERMINATOR_RANGES, ranges),
        StandardSet::Everything => ranges.push(CharacterRange::everything()),
    }
}

/// V8 `RangeContainsLatin1Equivalents`: U+039C, U+03BC and U+0178 have a Latin-1 case
/// equivalent (U+00B5, U+00FF).
#[must_use]
pub(crate) fn range_contains_latin1_equivalents(range: CharacterRange) -> bool {
    range.contains(0x039c) || range.contains(0x03bc) || range.contains(0x0178)
}

/// Adds the case equivalents of `i` without `u` (V8 `AddCaseEquivalents`, the ICU form): the
/// list is canonicalized first; the added ranges come after it, so the list is not canonical
/// at the end (V8 canonicalizes later). With `is_one_byte`, a range with no Latin-1
/// equivalent is clamped to U+00FF first.
pub fn add_case_equivalents(ranges: &mut Vec<CharacterRange>, is_one_byte: bool) {
    canonicalize(ranges);
    let mut others = Vec::new();
    for &range in ranges.iter() {
        let from = range.from;
        if from > MAX_UTF16_CODE_UNIT {
            continue;
        }
        let mut to = range.to.min(MAX_UTF16_CODE_UNIT);
        // Nothing to be done for surrogates.
        if from >= LEAD_SURROGATE_START && to <= TRAIL_SURROGATE_END {
            continue;
        }
        if is_one_byte && !range_contains_latin1_equivalents(range) {
            if from > MAX_ONE_BYTE_CHAR_CODE {
                continue;
            }
            to = to.min(MAX_ONE_BYTE_CHAR_CODE);
        }
        others.push(CharacterRange::range(from, to));
    }
    canonicalize(&mut others);
    let already_added = others.clone();
    special_case::close_over(&mut others, Mode::NonUnicode);
    ranges.extend(subtract(&others, &already_added));
}

/// Adds the case equivalents of `u` or `v` with `i` (V8 `AddUnicodeCaseEquivalents`): the
/// Simple_Case_Folding closure. `ranges` must be canonical; it is canonical after.
pub fn add_unicode_case_equivalents(ranges: &mut Vec<CharacterRange>) {
    debug_assert!(is_canonical(ranges));
    add_unicode_case_equivalents_to(ranges);
}

fn add_unicode_case_equivalents_to(ranges: &mut Vec<CharacterRange>) {
    canonicalize(ranges);
    // V8: skip the frequent {0, 0x10FFFF} range (crbug.com/v8/6727).
    if ranges.len() == 1 && ranges[0].is_everything(MAX_CODE_POINT) {
        return;
    }
    special_case::close_over(ranges, Mode::Unicode);
}

/// True if a code point of the canonical list is `c` (a binary search).
#[must_use]
pub fn contains(ranges: &[CharacterRange], c: u32) -> bool {
    let i = ranges.partition_point(|r| r.to < c);
    ranges.get(i).is_some_and(|r| r.from <= c)
}
