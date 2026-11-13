// Copyright 2019 the V8 project authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// Ported for ep9 from V8 src/regexp/regexp-compiler-tonode.cc (the ToNode functions, the
// unicode range splitter, the disjunction rewrites, the assertion sequence rewrite,
// Quantifier::ToNode) and src/regexp/regexp-compiler.cc (TextNode::CreateFor*,
// TextNode::CanMatchLatin1, Compiler::PreprocessRegExp) at
// 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0.
// The V8 licence text is in THIRD-PARTY-LICENSE (entry "V8 (BSD-3-Clause)").

//! Tree to graph (V8 `*::ToNode`). Each function takes the node that follows (`on_success`)
//! and gives the first node of its part, so a sequence is made from its end.
//!
//! ```text
//! Capture 0 of the whole tree -> accept           (PreprocessRegExp)
//!   + .*? before it, unless anchored or sticky    (the search loop; unrolled once with ^)
//! x{min,max} -> unrolled copies, or a LoopChoiceNode with a counter, guards, an empty check
//! [class] with u/v -> BMP | surrogate pairs | lone lead (?!trail) | (?<!lead) lone trail
//! \b \B with u/v + i -> look-arounds over the closed \w
//! $ with m -> (?=[\n\r  ]) | end
//! a|b|c -> sort runs of atoms, common prefix, single characters as a class
//! ```

use crate::ast::{AssertionType, StandardSet};
use crate::charset::{self, CharacterRange};
use crate::special_case::{self, Mode};

use super::Compiler;
use super::ir::{ClassRe, INFINITY, LookaroundRe, Re, TextRe};
use super::node::{
    Action, ActionNode, AssertionKind, AssertionNode, BackReferenceNode, ChoiceKind, ChoiceNode,
    ClassNode, End, Guard, GuardedAlternative, LoopInfo, NodeId, NodeKind, Relation, TextElement,
    TextElementKind, TextNode,
};
use super::{MAX_REGISTER, RECURSION_BUDGET};

const LEAD_SURROGATE_START: u32 = 0xd800;
const LEAD_SURROGATE_END: u32 = 0xdbff;
const TRAIL_SURROGATE_START: u32 = 0xdc00;
const TRAIL_SURROGATE_END: u32 = 0xdfff;
const NON_BMP_START: u32 = 0x10000;
const MAX_ONE_BYTE_CHAR_CODE: u32 = 0xff;

/// V8 `ExpansionLimiter::kMaxExpansionFactor`.
const MAX_EXPANSION_FACTOR: u64 = 6;
/// V8 `kMaxUnrolledMinMatches`.
const MAX_UNROLLED_MIN_MATCHES: u32 = 3;
/// V8 `kMaxUnrolledMaxMatches`.
const MAX_UNROLLED_MAX_MATCHES: u32 = 3;
/// V8 `kMaxRangesToInline`.
const MAX_RANGES_TO_INLINE: usize = 32;

fn lead_surrogate(c: u32) -> u32 {
    0xd800 + ((c - 0x10000) >> 10)
}

fn trail_surrogate(c: u32) -> u32 {
    0xdc00 + ((c - 0x10000) & 0x3ff)
}

fn case_mode(flags: crate::flags::Flags) -> Mode {
    if flags.either_unicode() {
        Mode::Unicode
    } else {
        Mode::NonUnicode
    }
}

/// V8 `CompareCaseInsensitive` as a sort key (`EquivalenceKey`).
fn first_char_key(re: &Re, ignore_case: bool, mode: Mode) -> u32 {
    let c = u32::from(re.first_atom()[0]);
    if ignore_case {
        special_case::equivalence_key(c, mode)
    } else {
        c
    }
}

/// V8 `Equals` of the disjunction rewrites.
fn chars_equal(ignore_case: bool, mode: Mode, a: u16, b: u16) -> bool {
    if a == b {
        return true;
    }
    ignore_case
        && special_case::equivalence_key(u32::from(a), mode)
            == special_case::equivalence_key(u32::from(b), mode)
}

/// The text node elements of one class (V8 `TextElement::FromClassRanges`).
fn class_element(ranges: Vec<CharacterRange>) -> TextElement {
    TextElement {
        cp_offset: 0,
        kind: TextElementKind::Class(ClassNode {
            ranges,
            negated: false,
            standard: None,
            contains_split_surrogate: false,
        }),
    }
}

/// V8 `ExpansionLimiter`: the state to restore and whether to expand.
struct Expansion {
    saved: u32,
    ok: bool,
}

/// V8 `Lookaround::Builder`.
struct LookaroundBuilder {
    positive: bool,
    on_success: NodeId,
    stack_pointer_register: u32,
    position_register: u32,
    on_match_success: NodeId,
}

impl Compiler {
    // ---- helpers ---------------------------------------------------------------------------

    fn text(
        &mut self,
        elements: Vec<TextElement>,
        read_backward: bool,
        on_success: NodeId,
    ) -> NodeId {
        self.add(NodeKind::Text(TextNode {
            elements,
            read_backward,
            on_success,
        }))
    }

    /// V8 `TextNode::CreateForCharacterRanges`.
    fn text_for_ranges(
        &mut self,
        ranges: Vec<CharacterRange>,
        read_backward: bool,
        on_success: NodeId,
    ) -> NodeId {
        self.text(vec![class_element(ranges)], read_backward, on_success)
    }

    /// V8 `TextNode::CreateForSurrogatePair` (lead range, trail ranges).
    fn text_for_surrogate_pair(
        &mut self,
        lead: CharacterRange,
        trail: Vec<CharacterRange>,
        read_backward: bool,
        on_success: NodeId,
    ) -> NodeId {
        let first = if lead.is_singleton() {
            TextElement {
                cp_offset: 0,
                kind: TextElementKind::Atom(vec![lead.from() as u16]),
            }
        } else {
            class_element(vec![lead])
        };
        self.text(vec![first, class_element(trail)], read_backward, on_success)
    }

    /// V8 `TextNode::CreateForSurrogatePair` (lead ranges, trail range).
    fn text_for_surrogate_pair_leads(
        &mut self,
        leads: Vec<CharacterRange>,
        trail: CharacterRange,
        read_backward: bool,
        on_success: NodeId,
    ) -> NodeId {
        self.text(
            vec![class_element(leads), class_element(vec![trail])],
            read_backward,
            on_success,
        )
    }

    fn action(&mut self, action: Action, on_success: NodeId) -> NodeId {
        self.add(NodeKind::Action(ActionNode { action, on_success }))
    }

    fn assertion(&mut self, kind: AssertionKind, on_success: NodeId) -> NodeId {
        self.add(NodeKind::Assertion(AssertionNode { kind, on_success }))
    }

    fn choice(&mut self, kind: ChoiceKind) -> NodeId {
        self.add(NodeKind::Choice(ChoiceNode {
            alternatives: Vec::with_capacity(2),
            not_at_start: false,
            do_not_inline: false,
            kind,
        }))
    }

    fn choice_mut(&mut self, id: NodeId) -> &mut ChoiceNode {
        match &mut self.node_mut(id).kind {
            NodeKind::Choice(c) => c,
            _ => unreachable!("a choice node"),
        }
    }

    fn add_alternative(&mut self, choice: NodeId, alt: GuardedAlternative) {
        self.choice_mut(choice).alternatives.push(alt);
    }

    fn is_backtrack(&self, id: NodeId) -> bool {
        self.node(id).is_backtrack()
    }

    /// V8 `TextNode::CanMatchLatin1` on the elements of a node that is about to be made.
    fn can_match_latin1(&self, elements: &[TextElement]) -> bool {
        let flags = self.flags;
        for elm in elements {
            match &elm.kind {
                TextElementKind::Atom(units) => {
                    for &c in units {
                        if !flags.ignore_case() {
                            if u32::from(c) > MAX_ONE_BYTE_CHAR_CODE {
                                return false;
                            }
                        } else {
                            let letters = special_case::case_independent_letters(
                                c,
                                flags.either_unicode(),
                                self.one_byte,
                            );
                            if letters
                                .first()
                                .is_none_or(|&l| u32::from(l) > MAX_ONE_BYTE_CHAR_CODE)
                            {
                                return false;
                            }
                        }
                    }
                }
                TextElementKind::Class(cr) => {
                    let mut ranges = cr.ranges.clone();
                    charset::canonicalize(&mut ranges);
                    let case_complications = || {
                        !flags.either_unicode()
                            && flags.ignore_case()
                            && ranges
                                .iter()
                                .any(|&r| charset::range_contains_latin1_equivalents(r))
                    };
                    if cr.negated {
                        if ranges
                            .first()
                            .is_some_and(|r| r.from() == 0 && r.to() >= MAX_ONE_BYTE_CHAR_CODE)
                            && !case_complications()
                        {
                            return false;
                        }
                    } else if ranges
                        .first()
                        .is_none_or(|r| r.from() > MAX_ONE_BYTE_CHAR_CODE)
                        && !case_complications()
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// A text node, or a backtrack when a Latin-1 subject cannot match it (V8
    /// `Atom::ToNodeImpl`, `Text::ToNodeImpl`).
    fn text_or_backtrack(&mut self, elements: Vec<TextElement>, on_success: NodeId) -> NodeId {
        if self.one_byte && !self.can_match_latin1(&elements) {
            return self.backtrack();
        }
        let rb = self.read_backward;
        self.text(elements, rb, on_success)
    }

    fn expansion_limiter(&mut self, factor: u64) -> Expansion {
        debug_assert!(factor > 0);
        let saved = self.current_expansion_factor;
        let mut ok = u64::from(saved) <= MAX_EXPANSION_FACTOR;
        if ok {
            if factor > MAX_EXPANSION_FACTOR {
                ok = false;
                self.current_expansion_factor = MAX_EXPANSION_FACTOR as u32 + 1;
            } else {
                let new_factor = u64::from(saved) * factor;
                ok = new_factor <= MAX_EXPANSION_FACTOR;
                self.current_expansion_factor = new_factor as u32;
            }
        }
        Expansion { saved, ok }
    }

    fn end_expansion(&mut self, e: &Expansion) {
        self.current_expansion_factor = e.saved;
    }

    fn lookaround_builder(
        &mut self,
        positive: bool,
        on_success: NodeId,
        stack_pointer_register: u32,
        position_register: u32,
        capture_register_count: u32,
        capture_register_start: u32,
    ) -> LookaroundBuilder {
        let on_match_success = if positive {
            self.action(
                Action::PositiveSubmatchSuccess {
                    stack_pointer_register,
                    position_register,
                    clear_register_count: capture_register_count,
                    clear_register_from: capture_register_start,
                },
                on_success,
            )
        } else {
            self.add(NodeKind::End(End::NegativeSubmatchSuccess {
                stack_pointer_register,
                position_register,
                clear_capture_count: capture_register_count,
                clear_capture_start: capture_register_start,
            }))
        };
        LookaroundBuilder {
            positive,
            on_success,
            stack_pointer_register,
            position_register,
            on_match_success,
        }
    }

    /// V8 `Lookaround::Builder::ForMatch`.
    fn lookaround_for_match(&mut self, b: &LookaroundBuilder, matcher: NodeId) -> NodeId {
        if b.positive {
            self.action(
                Action::BeginPositiveSubmatch {
                    stack_pointer_register: b.stack_pointer_register,
                    position_register: b.position_register,
                    success_node: b.on_match_success,
                },
                matcher,
            )
        } else {
            let choice = self.choice(ChoiceKind::NegativeLookaround);
            self.add_alternative(choice, GuardedAlternative::new(matcher));
            self.add_alternative(choice, GuardedAlternative::new(b.on_success));
            self.action(
                Action::BeginNegativeSubmatch {
                    stack_pointer_register: b.stack_pointer_register,
                    position_register: b.position_register,
                },
                choice,
            )
        }
    }

    // ---- ToNode ----------------------------------------------------------------------------

    /// V8 `Tree::ToNode`.
    pub(crate) fn node_of(&mut self, re: &Re, on_success: NodeId) -> NodeId {
        // Remove a whole branch that can only fail: its nodes would match and then backtrack.
        if self.is_backtrack(on_success) {
            return on_success;
        }
        if self.too_big {
            return self.backtrack();
        }
        match re {
            Re::Atom(units) => {
                let elements = vec![TextElement {
                    cp_offset: 0,
                    kind: TextElementKind::Atom(units.clone()),
                }];
                self.text_or_backtrack(elements, on_success)
            }
            Re::Text(elements) => {
                let elements = elements
                    .iter()
                    .map(|e| TextElement {
                        cp_offset: 0,
                        kind: match e {
                            TextRe::Atom(units) => TextElementKind::Atom(units.clone()),
                            TextRe::Class(c) => TextElementKind::Class(ClassNode {
                                ranges: c.ranges.clone(),
                                negated: c.negated,
                                standard: c.standard,
                                contains_split_surrogate: c.contains_split_surrogate,
                            }),
                        },
                    })
                    .collect();
                self.text_or_backtrack(elements, on_success)
            }
            Re::Class(c) => self.class_to_node(c, on_success),
            Re::ClassSet { body, .. } => match body {
                Some(body) => self.node_of(body, on_success),
                None => self.backtrack(),
            },
            Re::Disjunction(alts) => self.disjunction_to_node(alts.clone(), on_success),
            Re::Quantifier {
                min,
                max,
                greedy,
                body,
            } => self.quantifier_to_node(*min, *max, *greedy, body, on_success, false),
            Re::Assertion(t) => self.assertion_to_node(*t, on_success),
            Re::BackReference(captures) => {
                let mut node = on_success;
                // Only one of the captures can have matched, and a reference to a capture that
                // did not match is empty, so a reference to each of them is right.
                for &capture in captures.iter() {
                    self.backreferenced[capture as usize] = true;
                    let rb = self.read_backward;
                    node = self.add(NodeKind::BackReference(BackReferenceNode {
                        start_register: 2 * capture,
                        end_register: 2 * capture + 1,
                        read_backward: rb,
                        on_success: node,
                    }));
                }
                node
            }
            Re::Empty => on_success,
            Re::Group { flags, body } => {
                let previous = self.flags;
                self.flags = *flags;
                let node = self.node_of(body, on_success);
                self.flags = previous;
                node
            }
            Re::Lookaround(l) => self.lookaround_to_node(l, on_success),
            Re::Capture { index, body } => self.capture_to_node(body, Some(*index), on_success),
            Re::Alternative(nodes) => {
                let children = self.rewrite_assertion_sequence(nodes);
                let mut current = on_success;
                if self.read_backward {
                    for child in &children {
                        current = self.node_of(child, current);
                    }
                } else {
                    for child in children.iter().rev() {
                        current = self.node_of(child, current);
                    }
                }
                current
            }
        }
    }

    /// V8 `ClassRanges::ToNodeImpl`.
    fn class_to_node(&mut self, c: &ClassRe, on_success: NodeId) -> NodeId {
        let mut ranges = c.ranges.clone();
        charset::canonicalize(&mut ranges);
        let flags = self.flags;
        if flags.either_unicode() && flags.ignore_case() && !c.no_case_folding_needed {
            charset::add_unicode_case_equivalents(&mut ranges);
        }
        if !flags.either_unicode() || self.one_byte || c.contains_split_surrogate {
            let elements = vec![TextElement {
                cp_offset: 0,
                kind: TextElementKind::Class(ClassNode {
                    ranges,
                    negated: c.negated,
                    standard: c.standard,
                    contains_split_surrogate: c.contains_split_surrogate,
                }),
            }];
            return self.text_or_backtrack(elements, on_success);
        }
        if c.negated {
            // With v, classes are never negated here, but the empty class: V8 DCHECKs that.
            ranges = charset::negate(&ranges);
        }
        if ranges.is_empty() {
            return self.backtrack();
        }
        if c.standard == Some(StandardSet::Everything) {
            debug_assert!(!self.read_backward);
            return self.add(NodeKind::UnanchoredAdvance { on_success });
        }
        // Split the ranges: BMP code units, surrogate pairs (irregexp reads code units),
        // and lone surrogates (look-arounds keep them from matching half of a pair).
        let result = self.choice(ChoiceKind::Plain);
        let mut bmp = Vec::new();
        let mut lead = Vec::new();
        let mut trail = Vec::new();
        let mut non_bmp = Vec::new();
        for &r in &ranges {
            // V8 UnicodeRangeSplitter::AddRange.
            const PARTS: [(u32, u32, usize); 5] = [
                (0, LEAD_SURROGATE_START - 1, 0),
                (LEAD_SURROGATE_START, LEAD_SURROGATE_END, 1),
                (TRAIL_SURROGATE_START, TRAIL_SURROGATE_END, 2),
                (TRAIL_SURROGATE_END + 1, NON_BMP_START - 1, 0),
                (NON_BMP_START, 0x10_ffff, 3),
            ];
            for (start, end, target) in PARTS {
                if start > r.to() {
                    break;
                }
                let from = start.max(r.from());
                let to = end.min(r.to());
                if from > to {
                    continue;
                }
                let part = CharacterRange::range(from, to);
                match target {
                    0 => bmp.push(part),
                    1 => lead.push(part),
                    2 => trail.push(part),
                    _ => non_bmp.push(part),
                }
            }
        }
        let rb = self.read_backward;
        // AddBmpCharacters.
        if !bmp.is_empty() {
            charset::canonicalize(&mut bmp);
            let node = self.text_for_ranges(bmp, rb, on_success);
            self.add_alternative(result, GuardedAlternative::new(node));
        }
        // AddNonBmpSurrogatePairs.
        if !non_bmp.is_empty() {
            self.add_non_bmp_surrogate_pairs(result, on_success, non_bmp);
        }
        // AddLoneLeadSurrogates: \ud801 becomes \ud801(?![\udc00-\udfff]).
        if !lead.is_empty() {
            charset::canonicalize(&mut lead);
            let trails = vec![CharacterRange::range(
                TRAIL_SURROGATE_START,
                TRAIL_SURROGATE_END,
            )];
            let node = if rb {
                self.negative_lookaround_against_read_direction_and_match(
                    trails, lead, on_success, true,
                )
            } else {
                self.match_and_negative_lookaround_in_read_direction(
                    lead, trails, on_success, false,
                )
            };
            self.add_alternative(result, GuardedAlternative::new(node));
        }
        // AddLoneTrailSurrogates: \udc01 becomes (?<![\ud800-\udbff])\udc01.
        if !trail.is_empty() {
            charset::canonicalize(&mut trail);
            let leads = vec![CharacterRange::range(
                LEAD_SURROGATE_START,
                LEAD_SURROGATE_END,
            )];
            let node = if rb {
                self.match_and_negative_lookaround_in_read_direction(trail, leads, on_success, true)
            } else {
                self.negative_lookaround_against_read_direction_and_match(
                    leads, trail, on_success, false,
                )
            };
            self.add_alternative(result, GuardedAlternative::new(node));
        }
        if ranges.len() > MAX_RANGES_TO_INLINE {
            self.choice_mut(result).do_not_inline = true;
        }
        let choice = self.choice_mut(result);
        if choice.alternatives.len() == 1 {
            return choice.alternatives[0].node;
        }
        result
    }

    /// V8 `AddNonBmpSurrogatePairs`: one text node per lead range, `[lead][trails]`.
    fn add_non_bmp_surrogate_pairs(
        &mut self,
        result: NodeId,
        on_success: NodeId,
        mut non_bmp: Vec<CharacterRange>,
    ) {
        debug_assert!(!self.one_byte);
        // V8 groups by lead range in an unordered map; a BTreeMap gives one fixed order.
        let mut grouped: std::collections::BTreeMap<(u32, u32), Vec<CharacterRange>> =
            std::collections::BTreeMap::new();
        let mut full_trail: Vec<CharacterRange> = Vec::new();
        let mut add = |from_l: u32, to_l: u32, from_t: u32, to_t: u32| {
            if let Some(list) = grouped.get_mut(&(from_l, to_l)) {
                list.push(CharacterRange::range(from_t, to_t));
                return;
            }
            if from_t == TRAIL_SURROGATE_START && to_t == TRAIL_SURROGATE_END {
                full_trail.push(CharacterRange::range(from_l, to_l));
                return;
            }
            grouped.insert((from_l, to_l), vec![CharacterRange::range(from_t, to_t)]);
        };
        charset::canonicalize(&mut non_bmp);
        for r in &non_bmp {
            // E.g. [\u{10005}-\u{11005}] becomes \ud800[\udc05-\udfff] |
            // [\ud801-\ud803][\udc00-\udfff] | \ud804[\udc00-\udc05].
            let (from, to) = (r.from(), r.to());
            let mut from_l = lead_surrogate(from);
            let from_t = trail_surrogate(from);
            let mut to_l = lead_surrogate(to);
            let to_t = trail_surrogate(to);
            if from_l == to_l {
                add(from_l, to_l, from_t, to_t);
                continue;
            }
            if from_t != TRAIL_SURROGATE_START {
                add(from_l, from_l, from_t, TRAIL_SURROGATE_END);
                from_l += 1;
            }
            if to_t != TRAIL_SURROGATE_END {
                add(to_l, to_l, TRAIL_SURROGATE_START, to_t);
                to_l -= 1;
            }
            if from_l <= to_l {
                add(from_l, to_l, TRAIL_SURROGATE_START, TRAIL_SURROGATE_END);
            }
        }
        let rb = self.read_backward;
        if !full_trail.is_empty() {
            charset::canonicalize(&mut full_trail);
            let node = self.text_for_surrogate_pair_leads(
                full_trail,
                CharacterRange::range(TRAIL_SURROGATE_START, TRAIL_SURROGATE_END),
                rb,
                on_success,
            );
            self.add_alternative(result, GuardedAlternative::new(node));
        }
        for ((from_l, to_l), mut trails) in grouped {
            charset::canonicalize(&mut trails);
            let node = self.text_for_surrogate_pair(
                CharacterRange::range(from_l, to_l),
                trails,
                rb,
                on_success,
            );
            self.add_alternative(result, GuardedAlternative::new(node));
        }
    }

    /// V8 `NegativeLookaroundAgainstReadDirectionAndMatch`.
    fn negative_lookaround_against_read_direction_and_match(
        &mut self,
        lookbehind: Vec<CharacterRange>,
        matcher: Vec<CharacterRange>,
        on_success: NodeId,
        read_backward: bool,
    ) -> NodeId {
        let match_node = self.text_for_ranges(matcher, read_backward, on_success);
        let stack = self.unicode_lookaround_stack_register();
        let position = self.unicode_lookaround_position_register();
        let b = self.lookaround_builder(false, match_node, stack, position, 0, 0);
        let negative_match = self.text_for_ranges(lookbehind, !read_backward, b.on_match_success);
        self.lookaround_for_match(&b, negative_match)
    }

    /// V8 `MatchAndNegativeLookaroundInReadDirection`.
    fn match_and_negative_lookaround_in_read_direction(
        &mut self,
        matcher: Vec<CharacterRange>,
        lookahead: Vec<CharacterRange>,
        on_success: NodeId,
        read_backward: bool,
    ) -> NodeId {
        let stack = self.unicode_lookaround_stack_register();
        let position = self.unicode_lookaround_position_register();
        let b = self.lookaround_builder(false, on_success, stack, position, 0, 0);
        let negative_match = self.text_for_ranges(lookahead, read_backward, b.on_match_success);
        let after = self.lookaround_for_match(&b, negative_match);
        self.text_for_ranges(matcher, read_backward, after)
    }

    /// V8 `Disjunction::ToNodeImpl`, with the rewrites on a copy of the list.
    fn disjunction_to_node(&mut self, mut alternatives: Vec<Re>, on_success: NodeId) -> NodeId {
        if alternatives.len() > 2 {
            // V8 skips the sort and the prefix rewrite in look-behinds (`--regexp-optimization`
            // is on in V8 by default; it is not RegExpCompiler::optimize).
            if !self.read_backward && self.sort_consecutive_atoms(&mut alternatives) {
                alternatives = self.rationalize_consecutive_atoms(alternatives);
            }
            alternatives = self.fix_single_character_disjunctions(alternatives);
            if alternatives.len() == 1 {
                return self.node_of(&alternatives[0], on_success);
            }
        }
        let result = self.choice(ChoiceKind::Plain);
        for alt in &alternatives {
            let node = self.node_of(alt, on_success);
            if !self.is_backtrack(node) {
                self.add_alternative(result, GuardedAlternative::new(node));
            }
        }
        let choice = self.choice_mut(result);
        match choice.alternatives.len() {
            0 => self.backtrack(),
            1 => choice.alternatives[0].node,
            _ => result,
        }
    }

    /// V8 `Disjunction::SortConsecutiveAtoms`: a stable sort of each run of alternatives that
    /// start with an atom, by the first character (its equivalence key with `i`: changing
    /// `/is|I/` to `/I|is/` would change the result). True if a run has two or more.
    fn sort_consecutive_atoms(&self, alternatives: &mut [Re]) -> bool {
        let ignore_case = self.flags.ignore_case();
        let mode = case_mode(self.flags);
        let length = alternatives.len();
        let mut found = false;
        let mut i = 0;
        while i < length {
            while i < length && !alternatives[i].starts_with_atom() {
                i += 1;
            }
            if i == length {
                break;
            }
            let first_atom = i;
            i += 1;
            while i < length && alternatives[i].starts_with_atom() {
                i += 1;
            }
            alternatives[first_atom..i].sort_by_key(|a| first_char_key(a, ignore_case, mode));
            if i - first_atom > 1 {
                found = true;
            }
            // V8's `for` loop steps over the alternative that ended the run.
            i += 1;
        }
        found
    }

    /// V8 `Disjunction::RationalizeConsecutiveAtoms`: `ab|ac|az` becomes `a(?:b|c|z)`.
    fn rationalize_consecutive_atoms(&self, alternatives: Vec<Re>) -> Vec<Re> {
        let ignore_case = self.flags.ignore_case();
        let mode = case_mode(self.flags);
        let length = alternatives.len();
        let mut out = Vec::with_capacity(length);
        let mut i = 0;
        while i < length {
            if !alternatives[i].starts_with_atom() {
                out.push(alternatives[i].clone());
                i += 1;
                continue;
            }
            let atom = alternatives[i].first_atom();
            let common_prefix = atom[0];
            let first_with_prefix = i;
            let mut prefix_length = atom.len();
            i += 1;
            while i < length {
                if !alternatives[i].starts_with_atom() {
                    break;
                }
                let alt_atom = alternatives[i].first_atom();
                if !chars_equal(ignore_case, mode, alt_atom[0], common_prefix) {
                    break;
                }
                prefix_length = prefix_length.min(alt_atom.len());
                i += 1;
            }
            if i > first_with_prefix + 2 {
                // A run of 3 or more with a common first character; the common prefix may be
                // longer.
                let run_length = i - first_with_prefix;
                let alt_atom = alternatives[first_with_prefix].first_atom();
                for j in 1..run_length {
                    if prefix_length <= 1 {
                        break;
                    }
                    let old_atom = alternatives[j + first_with_prefix].first_atom();
                    for k in 1..prefix_length {
                        if !chars_equal(ignore_case, mode, alt_atom[k], old_atom[k]) {
                            prefix_length = k;
                            break;
                        }
                    }
                }
                let prefix = Re::Atom(alt_atom[..prefix_length].to_vec());
                let mut suffixes = Vec::with_capacity(run_length);
                for alt in &alternatives[first_with_prefix..i] {
                    match alt {
                        Re::Atom(old) => {
                            if old.len() == prefix_length {
                                suffixes.push(Re::Empty);
                            } else {
                                suffixes.push(Re::Atom(old[prefix_length..].to_vec()));
                            }
                        }
                        Re::Text(old_elements) => {
                            let mut new_elements = Vec::with_capacity(old_elements.len());
                            let old = alt.first_atom();
                            if old.len() != prefix_length {
                                new_elements.push(TextRe::Atom(old[prefix_length..].to_vec()));
                            }
                            new_elements.extend(old_elements[1..].iter().cloned());
                            if new_elements.is_empty() {
                                suffixes.push(Re::Empty);
                            } else {
                                suffixes.push(Re::Text(new_elements));
                            }
                        }
                        _ => unreachable!("starts_with_atom"),
                    }
                }
                out.push(Re::Alternative(vec![prefix, Re::Disjunction(suffixes)]));
            } else {
                out.extend(alternatives[first_with_prefix..i].iter().cloned());
            }
        }
        out
    }

    /// V8 `Disjunction::FixSingleCharacterDisjunctions`: `b|c|z` becomes `[bcz]`.
    fn fix_single_character_disjunctions(&self, alternatives: Vec<Re>) -> Vec<Re> {
        let flags = self.flags;
        let single = |re: &Re| -> Option<u16> {
            match re {
                Re::Atom(units) if units.len() == 1 => Some(units[0]),
                _ => None,
            }
        };
        let length = alternatives.len();
        let mut out = Vec::with_capacity(length);
        let mut i = 0;
        while i < length {
            let Some(c) = single(&alternatives[i]) else {
                out.push(alternatives[i].clone());
                i += 1;
                continue;
            };
            debug_assert!(!flags.either_unicode() || !(0xd800..=0xdbff).contains(&c));
            let mut contains_trail_surrogate = (0xdc00..=0xdfff).contains(&c);
            let first_in_run = i;
            i += 1;
            while i < length {
                let Some(c) = single(&alternatives[i]) else {
                    break;
                };
                contains_trail_surrogate |= (0xdc00..=0xdfff).contains(&c);
                i += 1;
            }
            if i > first_in_run + 1 {
                let ranges = alternatives[first_in_run..i]
                    .iter()
                    .map(|a| CharacterRange::singleton(u32::from(single(a).expect("a run"))))
                    .collect();
                out.push(Re::Class(ClassRe::new(
                    ranges,
                    false,
                    false,
                    flags.either_unicode() && contains_trail_surrogate,
                    false,
                )));
            } else {
                out.push(alternatives[first_in_run].clone());
            }
        }
        out
    }

    /// V8 `Quantifier::ToNode`. `x{f,t}` becomes:
    ///
    /// ```text
    ///             (r++)<-.
    ///               |     `
    ///               |     (x)
    ///               v     ^
    ///      (r=0)-->(?)---/ [if r < t]
    ///               |
    ///   [if r >= f] \----> ...
    /// ```
    pub(crate) fn quantifier_to_node(
        &mut self,
        min: u32,
        max: u32,
        greedy: bool,
        body: &Re,
        on_success: NodeId,
        not_at_start: bool,
    ) -> NodeId {
        if max == 0 {
            return on_success; // From the recursion below.
        }
        let body_can_be_empty = body.min_match() == 0;
        let mut body_start_reg = None;
        let capture_registers = body.capture_registers();
        // The captures of the body are cleared before each new iteration, so that
        // /(?:x(.)?z){2}/ on "xyzxz" captures "" rather than the "y" of the first one.
        let needs_capture_clearing = capture_registers.is_some() && max != 1;
        let want_unroll = self.optimize;
        if body_can_be_empty {
            body_start_reg = Some(self.allocate_register());
        } else if want_unroll && !needs_capture_clearing {
            // Unroll (x)+ and (x){3,}: the fixed copies, then the loop or the optional
            // copies after them.
            let limiter = self.expansion_limiter(u64::from(min) + u64::from(max != min));
            if min > 0 && min <= MAX_UNROLLED_MIN_MATCHES && limiter.ok {
                let new_max = if max == INFINITY { max } else { max - min };
                let mut answer =
                    self.quantifier_to_node(0, new_max, greedy, body, on_success, true);
                for _ in 0..min {
                    answer = self.node_of(body, answer);
                }
                self.end_expansion(&limiter);
                return answer;
            }
            self.end_expansion(&limiter);
            if max <= MAX_UNROLLED_MAX_MATCHES && min == 0 {
                // Unroll (x)? and (x){0,3}: one choice per optional copy.
                let limiter = self.expansion_limiter(u64::from(max));
                if limiter.ok {
                    let mut answer = on_success;
                    for _ in 0..max {
                        let alternation = self.choice(ChoiceKind::Plain);
                        let body_node = self.node_of(body, answer);
                        if greedy {
                            self.add_alternative(alternation, GuardedAlternative::new(body_node));
                            self.add_alternative(alternation, GuardedAlternative::new(on_success));
                        } else {
                            self.add_alternative(alternation, GuardedAlternative::new(on_success));
                            self.add_alternative(alternation, GuardedAlternative::new(body_node));
                        }
                        answer = alternation;
                        if not_at_start && !self.read_backward {
                            self.choice_mut(alternation).not_at_start = true;
                        }
                    }
                    self.end_expansion(&limiter);
                    return answer;
                }
                self.end_expansion(&limiter);
            }
        }
        let has_min = min > 0;
        let has_max = max < INFINITY;
        let needs_counter = has_min || has_max;
        let reg_ctr = if needs_counter {
            Some(self.allocate_register())
        } else {
            None
        };
        let rb = self.read_backward;
        let center = self.choice(ChoiceKind::Loop(LoopInfo {
            loop_node: None,
            continue_node: None,
            body_can_be_zero_length: body.min_match() == 0,
            read_backward: rb,
        }));
        if not_at_start && !rb {
            self.choice_mut(center).not_at_start = true;
        }
        let mut loop_return = center;
        if let Some(reg) = reg_ctr {
            loop_return = self.action(Action::IncrementRegister { register: reg }, loop_return);
        }
        if let Some(start) = body_start_reg {
            // The body can be empty: an empty iteration backtracks.
            loop_return = self.action(
                Action::EmptyMatchCheck {
                    start_register: start,
                    repetition_register: reg_ctr,
                    repetition_limit: min,
                },
                loop_return,
            );
        }
        let mut body_node = self.node_of(body, loop_return);
        if self.is_backtrack(body_node) {
            // The body never matches: with a minimum the whole part fails, else skip it.
            if has_min {
                return body_node;
            }
            return on_success;
        }
        if let Some(start) = body_start_reg {
            // Store the start position, so the empty check can compare.
            body_node = self.action(Action::RestorePosition { register: start }, body_node);
        }
        if let Some((from, to)) = capture_registers.filter(|_| needs_capture_clearing) {
            body_node = self.action(Action::ClearCaptures { from, to }, body_node);
        }
        let body_alt = GuardedAlternative {
            node: body_node,
            guard: has_max.then(|| Guard {
                register: reg_ctr.expect("a counter"),
                relation: Relation::Lt,
                value: max,
            }),
        };
        let rest_alt = GuardedAlternative {
            node: on_success,
            guard: has_min.then(|| Guard {
                register: reg_ctr.expect("a counter"),
                relation: Relation::Geq,
                value: min,
            }),
        };
        {
            let choice = self.choice_mut(center);
            if greedy {
                choice.alternatives.push(body_alt);
                choice.alternatives.push(rest_alt);
            } else {
                choice.alternatives.push(rest_alt);
                choice.alternatives.push(body_alt);
            }
            let ChoiceKind::Loop(info) = &mut choice.kind else {
                unreachable!("a loop choice");
            };
            info.loop_node = Some(body_alt.node);
            info.continue_node = Some(rest_alt.node);
        }
        let mut result = center;
        let body_min = body.min_match();
        if min > 0 && body_min > 0 && !rb {
            let eats = u64::from(min.min(256)) * u64::from(body_min.min(256));
            let eats = u8::try_from(eats).unwrap_or(u8::MAX);
            result = self.action(Action::EatsAtLeast { characters: eats }, result);
        }
        if let Some(reg) = reg_ctr {
            result = self.action(
                Action::SetRegisterForLoop {
                    register: reg,
                    value: 0,
                },
                result,
            );
        }
        result
    }

    /// V8 `Assertion::ToNodeImpl`.
    fn assertion_to_node(&mut self, t: AssertionType, on_success: NodeId) -> NodeId {
        let needs_unicode_case_equivalents =
            self.flags.either_unicode() && self.flags.ignore_case();
        match t {
            AssertionType::StartOfLine => self.assertion(AssertionKind::AfterNewline, on_success),
            AssertionType::StartOfInput => self.assertion(AssertionKind::AtStart, on_success),
            AssertionType::Boundary if needs_unicode_case_equivalents => {
                self.boundary_as_lookaround(true, on_success)
            }
            AssertionType::Boundary => self.assertion(AssertionKind::AtBoundary, on_success),
            AssertionType::NonBoundary if needs_unicode_case_equivalents => {
                self.boundary_as_lookaround(false, on_success)
            }
            AssertionType::NonBoundary => self.assertion(AssertionKind::AtNonBoundary, on_success),
            AssertionType::EndOfInput => self.assertion(AssertionKind::AtEnd, on_success),
            AssertionType::EndOfLine => {
                // $ with m: (?=[\n\r  ]) or the end of the input.
                let stack = self.allocate_register();
                let position = self.allocate_register();
                let result = self.choice(ChoiceKind::Plain);
                let submatch_success = self.action(
                    Action::PositiveSubmatchSuccess {
                        stack_pointer_register: stack,
                        position_register: position,
                        clear_register_count: 0,
                        clear_register_from: 0,
                    },
                    on_success,
                );
                // V8 ClassRanges(kLineTerminator). RX1's StandardSet has no line terminator
                // value: the class holds its ranges (the complement of `.`) and no mark.
                let not_line_terminator = ClassRe::standard(StandardSet::NotLineTerminator);
                let newline_matcher = self.text_for_ranges(
                    charset::negate(&not_line_terminator.ranges),
                    false,
                    submatch_success,
                );
                let end_of_line = self.action(
                    Action::BeginPositiveSubmatch {
                        stack_pointer_register: stack,
                        position_register: position,
                        success_node: submatch_success,
                    },
                    newline_matcher,
                );
                self.add_alternative(result, GuardedAlternative::new(end_of_line));
                let at_end = self.assertion(AssertionKind::AtEnd, on_success);
                self.add_alternative(result, GuardedAlternative::new(at_end));
                result
            }
        }
    }

    /// V8 `BoundaryAssertionAsLookaround`: `\b` is `(?<=\w)(?=\W)|(?<=\W)(?=\w)` and `\B`
    /// is `(?<=\w)(?=\w)|(?<=\W)(?=\W)`, over the case-closed `\w` of `u`/`v` with `i`.
    fn boundary_as_lookaround(&mut self, boundary: bool, on_success: NodeId) -> NodeId {
        let mut word_range = Vec::new();
        charset::add_class_escape(StandardSet::Word, &mut word_range, true);
        let stack = self.unicode_lookaround_stack_register();
        let position = self.unicode_lookaround_position_register();
        let result = self.choice(ChoiceKind::Plain);
        for i in 0..2 {
            let lookbehind_for_word = i == 0;
            let lookahead_for_word = boundary ^ lookbehind_for_word;
            let lookbehind =
                self.lookaround_builder(lookbehind_for_word, on_success, stack, position, 0, 0);
            let backward =
                self.text_for_ranges(word_range.clone(), true, lookbehind.on_match_success);
            let after_lookbehind = self.lookaround_for_match(&lookbehind, backward);
            let lookahead = self.lookaround_builder(
                lookahead_for_word,
                after_lookbehind,
                stack,
                position,
                0,
                0,
            );
            let forward =
                self.text_for_ranges(word_range.clone(), false, lookahead.on_match_success);
            let node = self.lookaround_for_match(&lookahead, forward);
            self.add_alternative(result, GuardedAlternative::new(node));
        }
        result
    }

    /// V8 `Lookaround::ToNodeImpl`.
    fn lookaround_to_node(&mut self, l: &LookaroundRe, on_success: NodeId) -> NodeId {
        let stack = self.allocate_register();
        let position = self.allocate_register();
        const REGISTERS_PER_CAPTURE: u32 = 2;
        const REGISTER_OF_FIRST_CAPTURE: u32 = 2;
        let register_count = l.capture_count * REGISTERS_PER_CAPTURE;
        let register_start = REGISTER_OF_FIRST_CAPTURE + l.capture_from * REGISTERS_PER_CAPTURE;
        let was_reading_backward = self.read_backward;
        self.read_backward = l.lookbehind;
        let b = self.lookaround_builder(
            l.positive,
            on_success,
            stack,
            position,
            register_count,
            register_start,
        );
        let matcher = self.node_of(&l.body, b.on_match_success);
        if self.is_backtrack(matcher) && (l.positive || self.too_big) {
            self.read_backward = was_reading_backward;
            return matcher;
        }
        let result = self.lookaround_for_match(&b, matcher);
        self.read_backward = was_reading_backward;
        result
    }

    /// V8 `Capture::ToNode` (`index` `None` is capture 0, the whole match).
    pub(crate) fn capture_to_node(
        &mut self,
        body: &Re,
        index: Option<u32>,
        on_success: NodeId,
    ) -> NodeId {
        let i = index.unwrap_or(0);
        let (mut start_reg, mut end_reg) = (2 * i, 2 * i + 1);
        if self.read_backward {
            std::mem::swap(&mut start_reg, &mut end_reg);
        }
        let store_end = self.action(
            Action::StorePosition {
                register: end_reg,
                capture: index,
            },
            on_success,
        );
        let body_node = self.node_of(body, store_end);
        if self.is_backtrack(body_node) {
            return body_node;
        }
        self.action(
            Action::StorePosition {
                register: start_reg,
                capture: index,
            },
            body_node,
        )
    }

    /// V8 `AssertionSequenceRewriter::MaybeRewrite`: flattens nested alternatives and groups
    /// with the same flags, drops a repeated assertion in a run of assertions, and makes a
    /// run with both `\b` and `\B` one class that matches nothing.
    fn rewrite_assertion_sequence(&self, terms: &[Re]) -> Vec<Re> {
        let flags = self.flags;
        let needs_rewrite = terms.iter().any(|t| match t {
            Re::Assertion(_) | Re::Alternative(_) => true,
            Re::Group { flags: f, .. } => *f == flags,
            _ => false,
        });
        if !needs_rewrite {
            return terms.to_vec();
        }
        let mut flattened = Vec::with_capacity(terms.len());
        let mut seen_assertions = 0u32;
        let mut sequence_start: Option<usize> = None;
        for t in terms {
            append_flattened_and_fold(
                &mut flattened,
                t,
                flags,
                &mut seen_assertions,
                &mut sequence_start,
            );
        }
        flattened
    }

    /// V8 `Compiler::PreprocessRegExp`: capture 0 around the tree, and the `.*?` search loop
    /// before it unless the tree is anchored at the start or the pattern is sticky.
    pub(crate) fn preprocess_regexp(&mut self, tree: &Re, contains_anchor: bool) -> NodeId {
        let accept = self.accept;
        let captured_body = self.capture_to_node(tree, None, accept);
        let mut node = captured_body;
        if !tree.is_certainly_anchored_at_start(RECURSION_BUDGET)
            && !self.flags.contains(crate::flags::Flags::STICKY)
        {
            self.has_search_prefix = true;
            let everything = Re::Class(ClassRe::standard(StandardSet::Everything));
            let loop_node = self.quantifier_to_node(
                0,
                INFINITY,
                false,
                &everything,
                captured_body,
                contains_anchor,
            );
            if contains_anchor {
                // Unroll the loop once for a match that may start at the start of the input.
                let first_step = self.choice(ChoiceKind::Plain);
                self.add_alternative(first_step, GuardedAlternative::new(captured_body));
                // A code point step in Unicode mode, so no match starts inside a pair.
                let step = self.node_of(&everything, loop_node);
                self.add_alternative(first_step, GuardedAlternative::new(step));
                node = first_step;
            } else {
                node = loop_node;
            }
        }
        debug_assert!(self.next_register <= MAX_REGISTER + 1);
        node
    }
}

/// V8 `AssertionSequenceRewriter::AppendFlattenedAndFold`.
fn append_flattened_and_fold(
    list: &mut Vec<Re>,
    term: &Re,
    flags: crate::flags::Flags,
    seen_assertions: &mut u32,
    sequence_start: &mut Option<usize>,
) {
    match term {
        Re::Assertion(t) => {
            let bit = 1u32 << assertion_bit(*t);
            if *seen_assertions & bit != 0 {
                return; // A repeated assertion.
            }
            *seen_assertions |= bit;
            let always_fails = (1 << assertion_bit(AssertionType::Boundary))
                | (1 << assertion_bit(AssertionType::NonBoundary));
            if *seen_assertions & always_fails == always_fails {
                let start = sequence_start.expect("a run of assertions");
                list.truncate(start);
                list.push(Re::Class(ClassRe::new(
                    Vec::new(),
                    false,
                    false,
                    false,
                    false,
                )));
                return;
            }
            if sequence_start.is_none() {
                *sequence_start = Some(list.len());
            }
            list.push(term.clone());
        }
        Re::Alternative(nodes) => {
            for n in nodes {
                append_flattened_and_fold(list, n, flags, seen_assertions, sequence_start);
            }
        }
        Re::Group { flags: f, body } if *f == flags => {
            append_flattened_and_fold(list, body, flags, seen_assertions, sequence_start);
        }
        _ => {
            *seen_assertions = 0;
            *sequence_start = None;
            list.push(term.clone());
        }
    }
}

/// V8 `Assertion::Type` values (the bit of each in the rewrite).
fn assertion_bit(t: AssertionType) -> u32 {
    match t {
        AssertionType::StartOfLine => 0,
        AssertionType::StartOfInput => 1,
        AssertionType::EndOfLine => 2,
        AssertionType::EndOfInput => 3,
        AssertionType::Boundary => 4,
        AssertionType::NonBoundary => 5,
    }
}
