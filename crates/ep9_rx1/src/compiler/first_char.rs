// Copyright 2019 the V8 project authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// Ported for ep9 from V8 src/regexp/regexp-compiler.cc (FirstCharacterSetBuilder,
// AppendClassRangesMatchSet, ComputeFirstCharacterSet, Compiler::ComputeQuickCheckFilters)
// at 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0.
// The V8 licence text is in THIRD-PARTY-LICENSE (entry "V8 (BSD-3-Clause)").

//! The first-character set and the quick check filters of an anchored pattern (V8
//! `ComputeQuickCheckFilters`): before it runs the program, the matcher may reject a start
//! position by the character there.
//!
//! The set is a superset of the characters that can start a match: a node that the walk
//! cannot read gives up the whole set (`None`), never an empty part, which would read as
//! "matches nothing" and reject real matches.

use crate::charset::{self, CharacterRange};

use super::Program;
use super::node::{Action, ChoiceKind, End, NodeId, NodeKind, TextElementKind};
use super::quick_check::QuickCheckDetails;

/// V8 `FirstCharacterSetBuilder::kBudget`.
const BUDGET: i32 = 500;
/// V8 `Compiler::kMaxRecursion` (the macOS value is 50; the filter is the same for either).
const MAX_RECURSION: u32 = 100;

/// V8 `RegExpData::kQuickCheckBitsetChars`: the reject bitset covers the Latin-1 units.
pub const QUICK_CHECK_BITSET_CHARS: u32 = 256;

/// The filters V8 stores on an anchored one-byte pattern (V8 `ComputeQuickCheckFilters`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuickCheckFilters {
    /// The quick check of the first 1 to 4 characters (mask, value), if it has a useful
    /// bit.
    pub mask_value: Option<(u32, u32)>,
    /// One bit per Latin-1 unit that cannot start a match, if any unit is rejected.
    pub reject_bitset: Option<[u32; 8]>,
}

struct Builder<'a> {
    program: &'a Program,
    out: Vec<CharacterRange>,
    budget: i32,
}

impl Builder<'_> {
    fn add_node(&mut self, id: NodeId, depth: u32) -> bool {
        if depth > MAX_RECURSION {
            return false;
        }
        self.budget -= 1;
        if self.budget < 0 {
            return false;
        }
        let program = self.program;
        let node = program.node(id);
        match &node.kind {
            // Zero width: the first consumed character is the successor's.
            NodeKind::Assertion(a) => self.add_node(a.on_success, depth + 1),
            NodeKind::Action(a) => match a.action {
                // Bookkeeping, and the success of a body that ate nothing (/^(?=a*)b/).
                Action::StorePosition { .. }
                | Action::ClearCaptures { .. }
                | Action::SetRegisterForLoop { .. }
                | Action::IncrementRegister { .. }
                | Action::EatsAtLeast { .. }
                | Action::PositiveSubmatchSuccess { .. } => self.add_node(a.on_success, depth + 1),
                // The body of a look-ahead matches at the position, the tighter answer; a
                // look-behind body gives up and the rest answers. Entries a failed walk left
                // only widen the set.
                Action::BeginPositiveSubmatch { success_node, .. } => {
                    if self.add_node(a.on_success, depth + 1) {
                        return true;
                    }
                    let after = program
                        .node(success_node)
                        .on_success()
                        .expect("a submatch success has a successor");
                    self.add_node(after, depth + 1)
                }
                // The body must fail: only the choice below (its continue side) counts.
                Action::BeginNegativeSubmatch { .. } => self.add_node(a.on_success, depth + 1),
                // Fail or move by run-time state.
                Action::EmptyMatchCheck { .. } | Action::RestorePosition { .. } => false,
            },
            NodeKind::Choice(c) => {
                if c.kind == ChoiceKind::NegativeLookaround {
                    return self
                        .add_node(c.alternatives[super::node::CONTINUE_INDEX].node, depth + 1);
                }
                // Any alternative may be taken (guards decide which, not the character).
                c.alternatives
                    .iter()
                    .all(|alt| self.add_node(alt.node, depth + 1))
            }
            NodeKind::Text(t) => {
                if t.read_backward {
                    return false;
                }
                let Some(elm) = t.elements.first() else {
                    return false;
                };
                match &elm.kind {
                    TextElementKind::Class(cr) => {
                        // The analysis added the case equivalents: the ranges are complete.
                        if cr.negated {
                            let mut positive = cr.ranges.clone();
                            charset::canonicalize(&mut positive);
                            self.out.extend(charset::negate(&positive));
                        } else {
                            self.out.extend(cr.ranges.iter().copied());
                        }
                        true
                    }
                    TextElementKind::Atom(data) => {
                        let Some(&c) = data.first() else {
                            return false;
                        };
                        if !node.flags.ignore_case() {
                            self.out.push(CharacterRange::singleton(u32::from(c)));
                            return true;
                        }
                        let letters = program.case_independent_letters(c, node.flags);
                        // No variant fits the subject form: a gap in what we know, not a
                        // proof of no match.
                        if letters.is_empty() {
                            return false;
                        }
                        self.out.extend(
                            letters
                                .iter()
                                .map(|&l| CharacterRange::singleton(u32::from(l))),
                        );
                        true
                    }
                }
            }
            // A backtrack fails, so nothing is exact; an accept matches without a character.
            NodeKind::End(End::Backtrack) => true,
            NodeKind::End(_) => false,
            // A back reference matches text known only at run time; an advance moves.
            NodeKind::BackReference(_) | NodeKind::UnanchoredAdvance { .. } => false,
        }
    }
}

impl Program {
    /// V8 `ComputeFirstCharacterSet`: the canonical set of the characters that can start a
    /// match at `id`, or `None` when it is not known.
    #[must_use]
    pub fn first_character_set(&self, id: NodeId) -> Option<Vec<CharacterRange>> {
        let mut b = Builder {
            program: self,
            out: Vec::new(),
            budget: BUDGET,
        };
        if !b.add_node(id, 0) {
            return None;
        }
        charset::canonicalize(&mut b.out);
        Some(b.out)
    }

    /// V8 `Compiler::ComputeQuickCheckFilters` for this program: only a one-byte program of a
    /// pattern with no search loop, without `m`, that eats at least one character.
    #[must_use]
    pub fn quick_check_filters(&self) -> QuickCheckFilters {
        let mut filters = QuickCheckFilters::default();
        if self.has_search_prefix() {
            return filters;
        }
        let start = self.start();
        let eats_at_least = self.eats_at_least(start, false);
        if !self.one_byte() || eats_at_least < 1 || self.flags().multiline() {
            return filters;
        }
        // The mask compares four characters loaded as one little-endian word.
        let chars = eats_at_least.min(4) as usize;
        let mut quick_check = QuickCheckDetails::new(chars);
        self.quick_check_details(start, &mut quick_check, 0, false, super::RECURSION_BUDGET);
        if !quick_check.cannot_match() {
            quick_check.rationalize(self.one_byte());
            if quick_check.mask() != 0 {
                filters.mask_value = Some((quick_check.mask(), quick_check.value()));
            }
        }
        // The inverse of the set: the matcher rejects on a set bit.
        if let Some(first_set) = self.first_character_set(start) {
            let mut accept = [0u32; 8];
            for range in &first_set {
                let to = range.to().min(QUICK_CHECK_BITSET_CHARS - 1);
                for c in range.from()..=to {
                    accept[(c / 32) as usize] |= 1 << (c % 32);
                }
            }
            let reject = accept.map(|w| !w);
            if reject.iter().any(|&w| w != 0) {
                filters.reject_bitset = Some(reject);
            }
        }
        filters
    }
}
