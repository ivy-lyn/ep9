// Copyright 2019 the V8 project authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// Ported for ep9 from V8 src/regexp/regexp-compiler.h (class QuickCheckDetails) and
// src/regexp/regexp-compiler.cc (QuickCheckDetails::Rationalize, Merge, Advance, Clear; the
// GetQuickCheckDetails of each node; ChoiceNode::CalculatePreloadCharacters;
// TextNode::GetCaseIndependentLetters) at 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0.
// The V8 licence text is in THIRD-PARTY-LICENSE (entry "V8 (BSD-3-Clause)").

//! The quick check (V8 `QuickCheckDetails`): for the next 1 to 4 characters a mask and a
//! value. The code generator loads the characters into one word, masks it and compares it;
//! a difference proves the node cannot match there, so it skips the full check.
//!
//! ```text
//! text 'ab'      position 0: mask ff value 61 (perfect)   position 1: mask ff value 62
//! text 'a' /i    'A' 41, 'a' 61 differ in bit 5: mask df value 41 (perfect: one free bit)
//! [0-7]          mask f8 value 30 (perfect: 8 characters = 2^3)
//! choice a|b     the merge of both: mask fd value 60 (not perfect)
//! ```

use crate::charset;
use crate::special_case;

use super::Program;
use super::node::{
    Action, AssertionKind, CONTINUE_INDEX, ChoiceKind, NodeId, NodeKind, TextElementKind,
};

/// V8 `QuickCheckDetails::kMaxPositions`.
pub const MAX_POSITIONS: usize = 4;

/// The largest Latin-1 code unit (V8 `String::kMaxOneByteCharCode`).
const MAX_ONE_BYTE_CHAR_CODE: u32 = 0xff;

/// V8 `CharMask`: 0xff for a one-byte program, 0xffff for two-byte.
#[must_use]
pub const fn char_mask(one_byte: bool) -> u32 {
    if one_byte { 0xff } else { 0xffff }
}

/// One character of a quick check (V8 `QuickCheckDetails::Position`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuickCheckPosition {
    /// The bits to compare.
    pub mask: u32,
    /// Their value.
    pub value: u32,
    /// A match of mask and value proves the character matches.
    pub determines_perfectly: bool,
    /// No character can match here.
    pub cannot_match: bool,
}

/// V8 `QuickCheckDetails`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuickCheckDetails {
    characters: usize,
    positions: [QuickCheckPosition; MAX_POSITIONS],
    mask: u32,
    value: u32,
}

impl QuickCheckDetails {
    /// Details for `characters` characters (at most 4, V8 `kMaxPositions`).
    #[must_use]
    pub fn new(characters: usize) -> QuickCheckDetails {
        assert!(characters <= MAX_POSITIONS);
        QuickCheckDetails {
            characters,
            ..QuickCheckDetails::default()
        }
    }

    /// The number of characters.
    #[must_use]
    pub const fn characters(&self) -> usize {
        self.characters
    }

    /// The details of character `i`.
    #[must_use]
    pub fn position(&self, i: usize) -> QuickCheckPosition {
        assert!(i < self.characters);
        self.positions[i]
    }

    fn position_mut(&mut self, i: usize) -> &mut QuickCheckPosition {
        assert!(i < self.characters);
        &mut self.positions[i]
    }

    /// The mask of all characters (set by [`QuickCheckDetails::rationalize`]).
    #[must_use]
    pub const fn mask(&self) -> u32 {
        self.mask
    }

    /// The value of all characters (set by [`QuickCheckDetails::rationalize`]).
    #[must_use]
    pub const fn value(&self) -> u32 {
        self.value
    }

    /// V8 `cannot_match`.
    #[must_use]
    pub fn cannot_match(&self) -> bool {
        self.positions[..self.characters]
            .iter()
            .any(|p| p.cannot_match)
    }

    /// V8 `set_cannot_match_from`.
    pub fn set_cannot_match_from(&mut self, index: usize) {
        for p in &mut self.positions[index..self.characters] {
            p.cannot_match = true;
        }
    }

    /// V8 `Rationalize`: packs the positions into one mask and value (8 bits per character
    /// one-byte, 16 two-byte). False if no position checks a bit of the low byte.
    pub fn rationalize(&mut self, one_byte: bool) -> bool {
        let mut found_useful_op = false;
        let char_mask = char_mask(one_byte);
        self.mask = 0;
        self.value = 0;
        let mut char_shift = 0;
        for p in &self.positions[..self.characters] {
            if p.mask & MAX_ONE_BYTE_CHAR_CODE != 0 {
                found_useful_op = true;
            }
            self.mask |= (p.mask & char_mask) << char_shift;
            self.value |= (p.value & char_mask) << char_shift;
            char_shift += if one_byte { 8 } else { 16 };
        }
        found_useful_op
    }

    /// V8 `Merge`: the check of either alternative (`self` or `other`) from `from_index`.
    pub fn merge(&mut self, other: &mut QuickCheckDetails, from_index: usize) {
        assert_eq!(self.characters, other.characters);
        for i in from_index..self.characters {
            let other_pos = &mut other.positions[i];
            let pos = &mut self.positions[i];
            if pos.cannot_match {
                *pos = *other_pos;
            } else if !other_pos.cannot_match {
                if pos.mask != other_pos.mask
                    || pos.value != other_pos.value
                    || !other_pos.determines_perfectly
                {
                    // The check is exact only when both sides make the same one.
                    pos.determines_perfectly = false;
                }
                pos.mask &= other_pos.mask;
                pos.value &= pos.mask;
                other_pos.value &= pos.mask;
                let differing_bits = pos.value ^ other_pos.value;
                pos.mask &= !differing_bits;
                pos.value &= pos.mask;
            }
        }
    }

    /// V8 `Advance`: drops the first `by` characters.
    pub fn advance(&mut self, by: usize) {
        if by >= self.characters {
            self.clear();
            return;
        }
        for i in 0..self.characters - by {
            self.positions[i] = self.positions[by + i];
        }
        for i in self.characters - by..self.characters {
            self.positions[i] = QuickCheckPosition::default();
        }
        self.characters -= by;
    }

    /// V8 `Clear`.
    pub fn clear(&mut self) {
        for p in &mut self.positions[..self.characters] {
            *p = QuickCheckPosition::default();
        }
        self.characters = 0;
    }
}

/// V8 `ChoiceNode::CalculatePreloadCharacters` for a code generator that can read unaligned
/// words: 4 or 2 one-byte characters (never 3), at most 2 two-byte.
#[must_use]
pub fn preload_characters(eats_at_least: u32, one_byte: bool) -> usize {
    let mut n = eats_at_least.min(4) as usize;
    if one_byte {
        if n == 3 {
            n = 2;
        }
    } else if n > 2 {
        n = 2;
    }
    n
}

/// Takes the left-most 1 bit and sets every bit to its right (V8 `SmearBitsRight`).
fn smear_bits_right(mut v: u32) -> u32 {
    v |= v >> 1;
    v |= v >> 2;
    v |= v >> 4;
    v |= v >> 8;
    v |= v >> 16;
    v
}

impl Program {
    /// V8 `TextNode::GetCaseIndependentLetters` for the node flags `flags`.
    pub(crate) fn case_independent_letters(&self, c: u16, flags: crate::Flags) -> Vec<u16> {
        special_case::case_independent_letters(c, flags.either_unicode(), self.one_byte())
    }

    /// V8 `Node::GetQuickCheckDetails`: fills `details` from `filled_in` on, for a match
    /// that starts at `id`.
    pub fn quick_check_details(
        &self,
        id: NodeId,
        details: &mut QuickCheckDetails,
        filled_in: usize,
        not_at_start: bool,
        budget: i32,
    ) {
        let node = self.node(id);
        match &node.kind {
            NodeKind::End(_) => details.set_cannot_match_from(filled_in),
            NodeKind::Action(a) => match a.action {
                Action::BeginPositiveSubmatch { success_node, .. } => {
                    // The node after the look-ahead, as for the eats-at-least value.
                    let after = self
                        .node(success_node)
                        .on_success()
                        .expect("a submatch success has a successor");
                    self.quick_check_details(after, details, filled_in, not_at_start, budget - 1);
                }
                // It rewinds the position; its eats-at-least value is 0, so no data.
                Action::PositiveSubmatchSuccess { .. } => {}
                _ => self.quick_check_details(
                    a.on_success,
                    details,
                    filled_in,
                    not_at_start,
                    budget - 1,
                ),
            },
            NodeKind::Assertion(a) => {
                if (a.kind == AssertionKind::AtStart && not_at_start)
                    || a.kind == AssertionKind::AtEnd
                {
                    details.set_cannot_match_from(filled_in);
                    return;
                }
                self.quick_check_details(
                    a.on_success,
                    details,
                    filled_in,
                    not_at_start,
                    budget - 1,
                );
            }
            // The text of a back reference is not known; an unanchored advance moves the
            // position by a variable amount.
            NodeKind::BackReference(_) | NodeKind::UnanchoredAdvance { .. } => {}
            NodeKind::Text(_) => {
                self.text_quick_check_details(id, details, filled_in, budget);
            }
            NodeKind::Choice(c) => match c.kind {
                ChoiceKind::NegativeLookaround => {
                    let next = c.alternatives[CONTINUE_INDEX].node;
                    self.quick_check_details(next, details, filled_in, not_at_start, budget - 1);
                }
                ChoiceKind::Loop(l) => {
                    if l.body_can_be_zero_length || budget <= 0 {
                        return;
                    }
                    self.choice_quick_check_details(
                        id,
                        details,
                        filled_in,
                        not_at_start || c.not_at_start,
                        budget,
                    );
                }
                ChoiceKind::Plain => {
                    self.choice_quick_check_details(id, details, filled_in, not_at_start, budget);
                }
            },
        }
    }

    /// V8 `ChoiceNode::GetQuickCheckDetails`.
    fn choice_quick_check_details(
        &self,
        id: NodeId,
        details: &mut QuickCheckDetails,
        filled_in: usize,
        not_at_start: bool,
        budget: i32,
    ) {
        let NodeKind::Choice(c) = &self.node(id).kind else {
            unreachable!("a choice")
        };
        let not_at_start = not_at_start || c.not_at_start;
        let count = c.alternatives.len();
        debug_assert!(count > 0);
        let budget = budget / count as i32;
        self.quick_check_details(
            c.alternatives[0].node,
            details,
            filled_in,
            not_at_start,
            budget,
        );
        for alt in &c.alternatives[1..] {
            let mut new_details = QuickCheckDetails::new(details.characters());
            self.quick_check_details(alt.node, &mut new_details, filled_in, not_at_start, budget);
            details.merge(&mut new_details, filled_in);
        }
    }

    /// V8 `TextNode::GetQuickCheckDetails`.
    fn text_quick_check_details(
        &self,
        id: NodeId,
        details: &mut QuickCheckDetails,
        mut filled_in: usize,
        budget: i32,
    ) {
        let node = self.node(id);
        let NodeKind::Text(t) = &node.kind else {
            unreachable!("a text node")
        };
        // A node that reads backward reads the other way than the quick check.
        if t.read_backward {
            return;
        }
        debug_assert!(filled_in < details.characters());
        let characters = details.characters();
        let char_mask = char_mask(self.one_byte());
        for elm in &t.elements {
            match &elm.kind {
                TextElementKind::Atom(quarks) => {
                    for &c in quarks.iter().take(characters) {
                        if node.flags.ignore_case() {
                            let chars = self.case_independent_letters(c, node.flags);
                            let pos = details.position_mut(filled_in);
                            if chars.is_empty() {
                                // Every case variant is above Latin-1 and the input is Latin-1.
                                pos.determines_perfectly = false;
                                details.set_cannot_match_from(filled_in);
                                return;
                            }
                            if chars.len() == 1 {
                                // No case equivalents: mask and compare decide.
                                pos.mask = char_mask;
                                pos.value = u32::from(chars[0]);
                                pos.determines_perfectly = true;
                            } else {
                                let mut common_bits = char_mask;
                                let mut bits = u32::from(chars[0]);
                                for &ch in &chars[1..] {
                                    let differing_bits = (u32::from(ch) & common_bits) ^ bits;
                                    common_bits ^= differing_bits;
                                    bits &= common_bits;
                                }
                                // Two letters that differ in one bit: mask and compare decide.
                                let one_zero = common_bits | !char_mask;
                                if chars.len() == 2
                                    && (!one_zero & (!one_zero).wrapping_sub(1)) == 0
                                {
                                    pos.determines_perfectly = true;
                                }
                                pos.mask = common_bits;
                                pos.value = bits;
                            }
                        } else {
                            let pos = details.position_mut(filled_in);
                            if u32::from(c) > char_mask {
                                pos.determines_perfectly = false;
                                details.set_cannot_match_from(filled_in);
                                return;
                            }
                            pos.mask = char_mask;
                            pos.value = u32::from(c);
                            pos.determines_perfectly = true;
                        }
                        filled_in += 1;
                        if filled_in == characters {
                            return;
                        }
                    }
                }
                TextElementKind::Class(cr) => {
                    let mut ranges = cr.ranges.clone();
                    // Canonical ranges: the perfect test below sums their sizes.
                    charset::canonicalize(&mut ranges);
                    let pos = details.position_mut(filled_in);
                    if cr.negated || ranges.is_empty() {
                        // No useful mask for a negated class (or an empty one).
                        pos.mask = 0;
                        pos.value = 0;
                    } else {
                        let Some(first_range) = ranges.iter().position(|r| r.from() <= char_mask)
                        else {
                            pos.determines_perfectly = false;
                            details.set_cannot_match_from(filled_in);
                            return;
                        };
                        let range = ranges[first_range];
                        let first_from = range.from();
                        let first_to = range.to().min(char_mask);
                        let mut total_characters = first_to - first_from + 1;
                        let differing_bits = first_from ^ first_to;
                        let mut common_bits = !smear_bits_right(differing_bits);
                        let mut bits = first_from & common_bits;
                        for range in &ranges[first_range + 1..] {
                            let from = range.from();
                            if from > char_mask {
                                continue;
                            }
                            let to = range.to().min(char_mask);
                            total_characters += to - from + 1;
                            let new_common_bits = !smear_bits_right(from ^ to);
                            common_bits &= new_common_bits;
                            bits &= new_common_bits;
                            let new_differing_bits = (from & common_bits) ^ bits;
                            common_bits ^= new_differing_bits;
                            bits &= common_bits;
                        }
                        pos.mask = common_bits;
                        pos.value = bits;
                        // Exact iff the class has 2^(free bits) characters.
                        let zero_bits = (!common_bits & char_mask).count_ones();
                        pos.determines_perfectly = u64::from(total_characters) == 1u64 << zero_bits;
                    }
                    filled_in += 1;
                    if filled_in == characters {
                        return;
                    }
                }
            }
        }
        debug_assert!(filled_in != details.characters());
        if !details.cannot_match() {
            self.quick_check_details(t.on_success, details, filled_in, true, budget - 1);
        }
    }

    /// The quick check of `characters` characters at `id` (V8 `EmitQuickCheck` up to the
    /// emit: the details, then `Rationalize`). `None` when V8 emits no quick check: no
    /// character, a position that cannot match, or no useful bit.
    #[must_use]
    pub fn quick_check(
        &self,
        id: NodeId,
        characters: usize,
        not_at_start: bool,
    ) -> Option<QuickCheckDetails> {
        if characters == 0 {
            return None;
        }
        let mut details = QuickCheckDetails::new(characters);
        self.quick_check_details(id, &mut details, 0, not_at_start, super::RECURSION_BUDGET);
        if details.cannot_match() || !details.rationalize(self.one_byte()) {
            return None;
        }
        Some(details)
    }
}
