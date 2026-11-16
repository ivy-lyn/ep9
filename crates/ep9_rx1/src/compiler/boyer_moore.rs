// Copyright 2019 the V8 project authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// Ported for ep9 from V8 src/regexp/regexp-compiler.h (BoyerMoorePositionInfo,
// BoyerMooreLookahead, FrequencyCollator) and src/regexp/regexp-compiler.cc (their methods,
// the FillInBMInfo of each node, the choice of EmitSkipInstructions,
// LoopChoiceNode::IsImplicitSearchLoop, TextNode::GetSuccessorOfOmnivorousTextNode) at
// 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0.
// The V8 licence text is in THIRD-PARTY-LICENSE (entry "V8 (BSD-3-Clause)").

//! The Boyer-Moore lookahead of the search loop (V8 `BoyerMooreLookahead`).
//!
//! For each of the next `length` positions, the set of code units (modulo 128) that a match
//! can have there. The search loop then reads the code unit at the end of the best interval
//! and, when it is in none of the sets of the interval, skips the whole interval.
//!
//! ```text
//! /..abcd.../   positions 0 1 2 3 4 5 6 7     all all a b c d all all
//!               the interval 2..5: if the unit at 5 is not one of a b c d, skip 4
//! ```

use crate::charset::CharacterRange;
use crate::special_case;

use super::Program;
use super::node::{Action, AssertionKind, ChoiceKind, NodeId, NodeKind, TextElementKind};

/// V8 `RegExpMacroAssembler::kTableSize`: the map has one bit per code unit modulo 128.
pub const TABLE_SIZE: usize = 128;
const TABLE_MASK: u32 = TABLE_SIZE as u32 - 1;

/// V8 `ContainedInLattice`: whether the units of a position are word characters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Lattice {
    #[default]
    NotYet = 0,
    In = 1,
    Out = 2,
    Unknown = 3,
}

impl Lattice {
    fn combine(self, other: Lattice) -> Lattice {
        match (self as u8) | (other as u8) {
            0 => Lattice::NotYet,
            1 => Lattice::In,
            2 => Lattice::Out,
            _ => Lattice::Unknown,
        }
    }
}

/// `\w` as `[from, to)` pairs and the end marker (V8 `kWordRanges`).
const WORD_RANGES: &[u32] = &[0x30, 0x3a, 0x41, 0x5b, 0x5f, 0x60, 0x61, 0x7b, 0x11_0000];

/// V8 `AddRange` of the lattice.
fn add_range(containment: Lattice, ranges: &[u32], from: u32, to: u32) -> Lattice {
    if containment == Lattice::Unknown {
        return containment;
    }
    let mut inside = false;
    let mut last = 0;
    for &bound in ranges {
        if bound <= from {
            inside = !inside;
            last = bound;
            continue;
        }
        // The new range is wholly inside last..bound.
        if last <= from && to < bound {
            return containment.combine(if inside { Lattice::In } else { Lattice::Out });
        }
        return Lattice::Unknown;
    }
    containment
}

/// V8 `BoyerMoorePositionInfo`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PositionInfo {
    map: u128,
    w: Lattice,
}

impl PositionInfo {
    /// True if code units `i` (modulo 128) can be at this position.
    #[must_use]
    pub const fn at(&self, i: usize) -> bool {
        self.map >> i & 1 != 0
    }

    /// The number of set bits.
    #[must_use]
    pub const fn map_count(&self) -> u32 {
        self.map.count_ones()
    }

    /// The bits, bit `i` for code units `i` modulo 128.
    #[must_use]
    pub const fn raw_bitset(&self) -> u128 {
        self.map
    }

    /// Every unit here is a word character.
    #[must_use]
    pub fn is_word(&self) -> bool {
        self.w == Lattice::In
    }

    /// No unit here is a word character.
    #[must_use]
    pub fn is_non_word(&self) -> bool {
        self.w == Lattice::Out
    }

    fn set_interval(&mut self, from: u32, to: u32) {
        self.w = add_range(self.w, WORD_RANGES, from, to);
        if to - from + 1 >= TABLE_SIZE as u32 {
            self.map = u128::MAX;
            return;
        }
        for i in from..=to {
            self.map |= 1 << (i & TABLE_MASK);
        }
    }

    fn set_all(&mut self) {
        self.w = Lattice::Unknown;
        self.map = u128::MAX;
    }
}

/// V8 `FrequencyCollator`: how often each code unit (modulo 128) is in a sample of the
/// subject. With no sample every frequency is 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrequencyCollator {
    counts: [u32; TABLE_SIZE],
    total_samples: u32,
}

impl Default for FrequencyCollator {
    fn default() -> FrequencyCollator {
        FrequencyCollator {
            counts: [0; TABLE_SIZE],
            total_samples: 0,
        }
    }
}

impl FrequencyCollator {
    /// V8 `kSampleSize`.
    pub const SAMPLE_SIZE: usize = 128;

    /// Counts the middle [`FrequencyCollator::SAMPLE_SIZE`] units of `subject`, as V8's
    /// `Compile` does with its sample subject.
    #[must_use]
    pub fn from_subject(subject: &[u16]) -> FrequencyCollator {
        let mut f = FrequencyCollator::default();
        let (start, end) = if subject.len() > Self::SAMPLE_SIZE {
            let start = (subject.len() - Self::SAMPLE_SIZE) / 2;
            (start, start + Self::SAMPLE_SIZE)
        } else {
            (0, subject.len())
        };
        for &c in &subject[start..end] {
            f.count_character(u32::from(c));
        }
        f
    }

    /// V8 `CountCharacter`.
    pub fn count_character(&mut self, c: u32) {
        self.counts[(c & TABLE_MASK) as usize] += 1;
        self.total_samples += 1;
    }

    /// V8 `Frequency`: in 128ths of the samples.
    #[must_use]
    pub fn frequency(&self, c: usize) -> u32 {
        if self.total_samples < 1 {
            return 1;
        }
        self.counts[c] * 128 / self.total_samples
    }
}

/// What the search loop does with a lookahead (V8 `EmitSkipInstructions`, the decision
/// without the code).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipPlan {
    /// No interval is worth a skip (the quick check does better).
    None,
    /// A position of the interval can hold no unit: backtrack.
    Backtrack,
    /// One position with one or two units that differ in at most one bit: skip until
    /// `(unit at max_lookahead) & mask == value` (V8 `SkipUntilCharAnd`).
    CharAnd {
        /// The offset of the unit to test.
        max_lookahead: u32,
        /// The value.
        value: u32,
        /// The mask (bits below 128 only; the two units' differing bit cleared).
        mask: u32,
    },
    /// Skip by `skip` while the unit at `max_lookahead` is not in `table` (V8
    /// `SkipUntilBitInTable`).
    BitInTable {
        /// The offset of the unit to test.
        max_lookahead: u32,
        /// The skip distance.
        skip: u32,
        /// One byte per unit modulo 128: 1 when the unit may start a match there.
        table: [u8; TABLE_SIZE],
        /// The same set as 16 rows of high-nibble bits (V8's SIMD table).
        nibble_table: [u8; 16],
    },
}

/// V8 `BoyerMooreLookahead`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoyerMooreLookahead {
    max_char: u32,
    bitmaps: Vec<PositionInfo>,
    one_byte: bool,
}

impl BoyerMooreLookahead {
    /// An empty lookahead of `length` positions.
    #[must_use]
    pub fn new(length: usize, one_byte: bool) -> BoyerMooreLookahead {
        BoyerMooreLookahead {
            max_char: if one_byte { 0xff } else { 0xffff },
            bitmaps: vec![PositionInfo::default(); length],
            one_byte,
        }
    }

    /// The number of positions.
    #[must_use]
    pub fn length(&self) -> usize {
        self.bitmaps.len()
    }

    /// The largest code unit of the subject form.
    #[must_use]
    pub const fn max_char(&self) -> u32 {
        self.max_char
    }

    /// Position `i`.
    #[must_use]
    pub fn at(&self, i: usize) -> &PositionInfo {
        &self.bitmaps[i]
    }

    /// V8 `Count`.
    #[must_use]
    pub fn count(&self, i: usize) -> u32 {
        self.bitmaps[i].map_count()
    }

    fn set(&mut self, i: usize, c: u32) {
        if c > self.max_char {
            return;
        }
        self.bitmaps[i].set_interval(c, c);
    }

    fn set_interval(&mut self, i: usize, from: u32, to: u32) {
        if from > self.max_char {
            return;
        }
        self.bitmaps[i].set_interval(from, to.min(self.max_char));
    }

    fn set_all(&mut self, i: usize) {
        self.bitmaps[i].set_all();
    }

    /// V8 `SetRest`.
    fn set_rest(&mut self, from: usize) {
        for i in from..self.bitmaps.len() {
            self.set_all(i);
        }
    }

    /// V8 `FindWorthwhileInterval`: the longest interval of the fewest units per position.
    #[must_use]
    pub fn find_worthwhile_interval(
        &self,
        frequencies: &FrequencyCollator,
    ) -> Option<(usize, usize)> {
        let mut biggest_points = 0;
        let mut best = (0, 0);
        // More than 32 of 128 units at a position: a skip is unlikely to pay.
        const MAX_MAX: u32 = 32;
        let mut max_number_of_chars = 4;
        while max_number_of_chars < MAX_MAX {
            biggest_points = self.find_best_interval(
                max_number_of_chars,
                biggest_points,
                &mut best,
                frequencies,
            );
            max_number_of_chars *= 2;
        }
        (biggest_points != 0).then_some(best)
    }

    /// V8 `FindBestInterval`: points = width times the chance to skip (by the frequencies).
    fn find_best_interval(
        &self,
        max_number_of_chars: u32,
        old_biggest_points: i64,
        best: &mut (usize, usize),
        frequencies: &FrequencyCollator,
    ) -> i64 {
        let mut biggest_points = old_biggest_points;
        let length = self.length();
        let mut i = 0;
        while i < length {
            while i < length && self.count(i) > max_number_of_chars {
                i += 1;
            }
            if i == length {
                break;
            }
            let remembered_from = i;
            let mut union_bitset = 0u128;
            while i < length && self.count(i) <= max_number_of_chars {
                union_bitset |= self.bitmaps[i].raw_bitset();
                i += 1;
            }
            let mut frequency: i64 = 0;
            while union_bitset != 0 {
                let j = union_bitset.trailing_zeros() as usize;
                // +1: a small boost per character when the sample is too small.
                frequency += i64::from(frequencies.frequency(j)) + 1;
                union_bitset &= union_bitset - 1;
            }
            // By halving we switch off the skip when its chance is under 50%: the quick
            // check does better then.
            let in_quickcheck_range = (i - remembered_from < 4)
                || if self.one_byte {
                    remembered_from <= 4
                } else {
                    remembered_from <= 2
                };
            let size = TABLE_SIZE as i64;
            let probability = if in_quickcheck_range { size / 2 } else { size } - frequency;
            let points = (i - remembered_from) as i64 * probability;
            if points > biggest_points {
                *best = (remembered_from, i - 1);
                biggest_points = points;
            }
        }
        biggest_points
    }

    /// V8 `GetSkipTable`: the units that do not stop a match at offsets
    /// `min_lookahead..=max_lookahead`, as a 128-byte table and a 16-byte nibble table, and
    /// the skip distance.
    #[must_use]
    pub fn skip_table(
        &self,
        min_lookahead: usize,
        max_lookahead: usize,
    ) -> ([u8; TABLE_SIZE], [u8; 16], u32) {
        let mut table = [0u8; TABLE_SIZE];
        let mut nibbles = [0u8; 16];
        for i in (min_lookahead..=max_lookahead).rev() {
            let mut bitset = self.bitmaps[i].raw_bitset();
            while bitset != 0 {
                let j = bitset.trailing_zeros() as usize;
                table[j] = 1;
                nibbles[j & 0x0f] |= 1 << ((j >> 4) & 0x07);
                bitset &= bitset - 1;
            }
        }
        (table, nibbles, (max_lookahead + 1 - min_lookahead) as u32)
    }

    /// The choice of V8 `EmitSkipInstructions` (with `SkipUntilCharAnd` available, as on
    /// every target with the SIMD scan).
    #[must_use]
    pub fn skip_plan(&self, frequencies: &FrequencyCollator) -> SkipPlan {
        let Some((min_lookahead, max_lookahead)) = self.find_worthwhile_interval(frequencies)
        else {
            return SkipPlan::None;
        };
        // One position with one or two units?
        let mut found_single_position = false;
        let mut char_one = 0u32;
        let mut char_two = 0u32;
        for i in min_lookahead..=max_lookahead {
            let map = &self.bitmaps[i];
            if map.map_count() == 0 {
                // No unit can be here; another alternative or start may still match.
                return SkipPlan::Backtrack;
            }
            if found_single_position || map.map_count() > 2 {
                found_single_position = false;
                break;
            }
            let mut bitset = map.raw_bitset();
            char_one = bitset.trailing_zeros();
            if map.map_count() == 2 {
                bitset &= bitset - 1;
                char_two = bitset.trailing_zeros();
            } else {
                char_two = char_one;
            }
            if (char_one ^ char_two).count_ones() > 1 {
                // No good test for two units that differ in more than one bit.
                break;
            }
            found_single_position = true;
        }
        if found_single_position && max_lookahead < 3 {
            // The mask and compare does better.
            return SkipPlan::None;
        }
        if found_single_position {
            let mask = TABLE_MASK & !(char_one ^ char_two);
            return SkipPlan::CharAnd {
                max_lookahead: max_lookahead as u32,
                value: char_one & mask,
                mask,
            };
        }
        let (table, nibble_table, skip) = self.skip_table(min_lookahead, max_lookahead);
        SkipPlan::BitInTable {
            max_lookahead: max_lookahead as u32,
            skip,
            table,
            nibble_table,
        }
    }
}

impl Program {
    /// V8 `Node::FillInBMInfo`: adds what a match from `id` can have at offsets `offset..`.
    pub fn fill_in_bm_info(
        &self,
        id: NodeId,
        offset: usize,
        budget: i32,
        bm: &mut BoyerMooreLookahead,
        not_at_start: bool,
    ) {
        let node = self.node(id);
        match &node.kind {
            NodeKind::End(_) | NodeKind::UnanchoredAdvance { .. } => {}
            NodeKind::Action(a) => match a.action {
                Action::BeginPositiveSubmatch { success_node, .. } => {
                    let after = self
                        .node(success_node)
                        .on_success()
                        .expect("a submatch success has a successor");
                    self.fill_in_bm_info(after, offset, budget - 1, bm, not_at_start);
                }
                // It rewinds the position (eats-at-least 0): no data.
                Action::PositiveSubmatchSuccess { .. } => {}
                _ => self.fill_in_bm_info(a.on_success, offset, budget - 1, bm, not_at_start),
            },
            NodeKind::Assertion(a) => {
                if a.kind == AssertionKind::AtStart && not_at_start {
                    return;
                }
                self.fill_in_bm_info(a.on_success, offset, budget - 1, bm, not_at_start);
            }
            NodeKind::BackReference(_) => {
                // Too hard: any unit.
                bm.set_rest(offset);
            }
            NodeKind::Choice(c) => match c.kind {
                ChoiceKind::NegativeLookaround => {
                    let next = c.alternatives[super::node::CONTINUE_INDEX].node;
                    self.fill_in_bm_info(next, offset, budget - 1, bm, not_at_start);
                }
                ChoiceKind::Loop(l) => {
                    if l.body_can_be_zero_length || budget <= 0 {
                        bm.set_rest(offset);
                        return;
                    }
                    self.choice_fill_in_bm_info(id, offset, budget - 1, bm, not_at_start);
                }
                ChoiceKind::Plain => {
                    self.choice_fill_in_bm_info(id, offset, budget, bm, not_at_start);
                }
            },
            NodeKind::Text(t) => {
                if offset >= bm.length() || t.read_backward {
                    return;
                }
                let mut offset = offset;
                let max_char = bm.max_char();
                for elm in &t.elements {
                    if offset >= bm.length() {
                        return;
                    }
                    match &elm.kind {
                        TextElementKind::Atom(units) => {
                            for &c in units {
                                if offset >= bm.length() {
                                    return;
                                }
                                if node.flags.ignore_case() {
                                    for l in special_case::case_independent_letters(
                                        c,
                                        node.flags.either_unicode(),
                                        self.one_byte(),
                                    ) {
                                        bm.set(offset, u32::from(l));
                                    }
                                } else if u32::from(c) <= max_char {
                                    bm.set(offset, u32::from(c));
                                }
                                offset += 1;
                            }
                        }
                        TextElementKind::Class(cr) => {
                            if cr.negated {
                                bm.set_all(offset);
                            } else {
                                for r in &cr.ranges {
                                    let r: CharacterRange = *r;
                                    if r.from() > max_char {
                                        continue;
                                    }
                                    bm.set_interval(offset, r.from(), r.to().min(max_char));
                                }
                            }
                            offset += 1;
                        }
                    }
                }
                if offset >= bm.length() {
                    return;
                }
                // Not at the start after a text node.
                self.fill_in_bm_info(t.on_success, offset, budget - 1, bm, true);
            }
        }
    }

    /// V8 `ChoiceNode::FillInBMInfo`.
    fn choice_fill_in_bm_info(
        &self,
        id: NodeId,
        offset: usize,
        budget: i32,
        bm: &mut BoyerMooreLookahead,
        not_at_start: bool,
    ) {
        let NodeKind::Choice(c) = &self.node(id).kind else {
            unreachable!("a choice")
        };
        let budget = (budget - 1) / c.alternatives.len() as i32;
        for alt in &c.alternatives {
            if alt.guard.is_some() {
                bm.set_rest(offset); // Give up.
                return;
            }
            self.fill_in_bm_info(alt.node, offset, budget, bm, not_at_start);
        }
    }

    /// V8 `TextNode::GetSuccessorOfOmnivorousTextNode`: the successor of a forward text node
    /// of one class that matches every code unit of the subject form.
    #[must_use]
    pub fn successor_of_omnivorous_text_node(&self, id: NodeId) -> Option<NodeId> {
        let NodeKind::Text(t) = &self.node(id).kind else {
            return None;
        };
        if t.read_backward || t.elements.len() != 1 {
            return None;
        }
        let TextElementKind::Class(cr) = &t.elements[0].kind else {
            return None;
        };
        let mut ranges = cr.ranges.clone();
        crate::charset::canonicalize(&mut ranges);
        if cr.negated {
            return ranges.is_empty().then_some(t.on_success);
        }
        let max_char = super::quick_check::char_mask(self.one_byte());
        (ranges.len() == 1 && ranges[0].is_everything(max_char)).then_some(t.on_success)
    }

    /// V8 `LoopChoiceNode::IsImplicitSearchLoop`: two alternatives, the second an unguarded
    /// omnivorous text node back to the loop.
    #[must_use]
    pub fn is_implicit_search_loop(&self, id: NodeId) -> bool {
        let NodeKind::Choice(c) = &self.node(id).kind else {
            return false;
        };
        if !matches!(c.kind, ChoiceKind::Loop(_)) || c.alternatives.len() != 2 {
            return false;
        }
        let alt1 = c.alternatives[1];
        alt1.guard.is_none() && self.successor_of_omnivorous_text_node(alt1.node) == Some(id)
    }

    /// The lookahead V8's `EmitOptimizedUnanchoredSearch` builds for the search loop `id`:
    /// `min(8, eats-at-least)` positions of alternative 0. `None` when `id` is not the
    /// search loop or the pattern eats nothing.
    #[must_use]
    pub fn search_loop_lookahead(&self, id: NodeId) -> Option<BoyerMooreLookahead> {
        if !self.is_implicit_search_loop(id) {
            return None;
        }
        let eats = super::MAX_LOOKAHEAD_FOR_BOYER_MOORE.min(self.eats_at_least(id, false));
        if eats < 1 {
            return None;
        }
        let mut bm = BoyerMooreLookahead::new(eats as usize, self.one_byte());
        let NodeKind::Choice(c) = &self.node(id).kind else {
            unreachable!("a loop")
        };
        self.fill_in_bm_info(
            c.alternatives[0].node,
            0,
            super::RECURSION_BUDGET,
            &mut bm,
            false,
        );
        Some(bm)
    }
}
