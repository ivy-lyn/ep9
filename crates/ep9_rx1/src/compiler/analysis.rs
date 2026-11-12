// Copyright 2019 the V8 project authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// Ported for ep9 from V8 src/regexp/regexp-compiler.cc (class Analysis,
// AssertionPropagator, EatsAtLeastPropagator, TextNode::MakeCaseIndependent,
// TextNode::CalculateOffsets, ClassRanges::is_standard of regexp-compiler-tonode.cc) at
// 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0.
// The V8 licence text is in THIRD-PARTY-LICENSE (entry "V8 (BSD-3-Clause)").

//! The analysis (V8 `AnalyzeRegExp`): one walk from the start node that
//!
//! - adds the case equivalents of `i` without `u`/`v` to the classes of text nodes
//!   (`MakeCaseIndependent`; with `u`/`v` the node maker did it),
//! - sets the offsets of text elements (`CalculateOffsets`),
//! - propagates the assertion interests (`AssertionPropagator`) and the eats-at-least
//!   values (`EatsAtLeastPropagator`) from each node's successors to the node.
//!
//! V8 recurses (`EnsureAnalyzed`), visits each node once, and reads the partial value of a
//! node that is still being analyzed when a loop returns to it. Here an explicit stack runs
//! the same visits in the same order, so a deep graph cannot overflow the native stack.

use crate::charset::{self, CharacterRange};

use super::node::{
    Action, AssertionKind, CONTINUE_INDEX, ChoiceKind, EatsAtLeastInfo, LOOKAROUND_INDEX, Node,
    NodeId, NodeKind, TextElementKind,
};

/// The half-open pair tables of `regexp-compiler.h` that `ClassRanges::is_standard`
/// compares a class with.
const SPACE_RANGES: &[u32] = &[
    0x09, 0x0e, 0x20, 0x21, 0xa0, 0xa1, 0x1680, 0x1681, 0x2000, 0x200b, 0x2028, 0x202a, 0x202f,
    0x2030, 0x205f, 0x2060, 0x3000, 0x3001, 0xfeff, 0xff00,
];
const WORD_RANGES: &[u32] = &[0x30, 0x3a, 0x41, 0x5b, 0x5f, 0x60, 0x61, 0x7b];
const LINE_TERMINATOR_RANGES: &[u32] = &[0x0a, 0x0b, 0x0d, 0x0e, 0x2028, 0x202a];
const RANGE_END_MARKER: u32 = 0x11_0000;

/// V8 `CompareRanges`: the canonical list equals the pair table.
fn compare_ranges(ranges: &[CharacterRange], table: &[u32]) -> bool {
    if ranges.len() * 2 != table.len() {
        return false;
    }
    ranges
        .iter()
        .zip(table.chunks(2))
        .all(|(r, pair)| r.from() == pair[0] && r.to() + 1 == pair[1])
}

/// V8 `CompareInverseRanges`: the canonical list is the complement of the pair table.
fn compare_inverse_ranges(ranges: &[CharacterRange], table: &[u32]) -> bool {
    if ranges.len() * 2 != table.len() + 2 {
        return false;
    }
    let mut from = 0;
    for (i, r) in ranges.iter().enumerate() {
        if r.from() != from {
            return false;
        }
        let end = table.get(i * 2).copied().unwrap_or(RANGE_END_MARKER);
        if r.to() + 1 != end {
            return false;
        }
        from = table.get(i * 2 + 1).copied().unwrap_or(RANGE_END_MARKER);
    }
    from == RANGE_END_MARKER
}

/// V8 `ClassRanges::is_standard` (not negated, and a standard set or equal to one of the
/// tables).
fn is_standard(class: &super::node::ClassNode) -> bool {
    if class.negated {
        return false;
    }
    if class.standard.is_some() {
        return true;
    }
    let mut ranges = class.ranges.clone();
    charset::canonicalize(&mut ranges);
    compare_ranges(&ranges, SPACE_RANGES)
        || compare_inverse_ranges(&ranges, SPACE_RANGES)
        || compare_inverse_ranges(&ranges, LINE_TERMINATOR_RANGES)
        || compare_ranges(&ranges, LINE_TERMINATOR_RANGES)
        || compare_ranges(&ranges, WORD_RANGES)
        || compare_inverse_ranges(&ranges, WORD_RANGES)
}

/// V8 `TextNode::MakeCaseIndependent`.
fn make_case_independent(node: &mut Node, one_byte: bool) {
    let flags = node.flags;
    if !flags.ignore_case() || flags.either_unicode() {
        return;
    }
    let NodeKind::Text(t) = &mut node.kind else {
        return;
    };
    for elm in &mut t.elements {
        if let TextElementKind::Class(c) = &mut elm.kind {
            // The standard classes are the same under i, and it is slower to not know it.
            if is_standard(c) {
                continue;
            }
            charset::add_case_equivalents(&mut c.ranges, one_byte);
        }
    }
}

/// V8 `TextNode::CalculateOffsets`.
fn calculate_offsets(node: &mut Node) {
    let NodeKind::Text(t) = &mut node.kind else {
        return;
    };
    let mut cp_offset = 0;
    for elm in &mut t.elements {
        elm.cp_offset = cp_offset;
        cp_offset += elm.kind.length();
    }
}

/// One step of the visit of a node (the points of V8's `Visit*` between its calls of
/// `EnsureAnalyzed`).
#[derive(Clone, Copy, Debug)]
enum Step {
    /// Enter the node (V8 `EnsureAnalyzed`).
    Ensure(NodeId),
    /// A sequence node: its successor is analyzed.
    AfterSuccessor(NodeId),
    /// A plain choice: alternative `i` is analyzed.
    AfterAlternative(NodeId, usize),
    /// A loop: the continue node is analyzed.
    AfterLoopContinue(NodeId),
    /// A loop: the loop node is analyzed.
    AfterLoopBody(NodeId),
    /// A negative look-around: the look-around body is analyzed.
    AfterLookaround(NodeId),
    /// A negative look-around: the continue node is analyzed.
    AfterContinue(NodeId),
    /// The node is done.
    Done(NodeId),
}

fn eats(nodes: &[Node], id: NodeId) -> EatsAtLeastInfo {
    nodes[id.index()].eats
}

fn sat(v: u32) -> u8 {
    u8::try_from(v).unwrap_or(u8::MAX)
}

/// V8 `AnalyzeRegExp`.
pub(crate) fn analyze(nodes: &mut [Node], start: NodeId, one_byte: bool) {
    let mut stack = vec![Step::Ensure(start)];
    while let Some(step) = stack.pop() {
        match step {
            Step::Ensure(id) => {
                let info = &mut nodes[id.index()].info;
                if info.been_analyzed || info.being_analyzed {
                    continue;
                }
                info.being_analyzed = true;
                stack.push(Step::Done(id));
                enter(nodes, id, one_byte, &mut stack);
            }
            Step::AfterSuccessor(id) => after_successor(nodes, id),
            Step::AfterAlternative(id, i) => {
                visit_choice(nodes, id, i);
                let NodeKind::Choice(c) = &nodes[id.index()].kind else {
                    unreachable!("a choice")
                };
                if let Some(next) = c.alternatives.get(i + 1) {
                    stack.push(Step::AfterAlternative(id, i + 1));
                    stack.push(Step::Ensure(next.node));
                }
            }
            Step::AfterLoopContinue(id) => {
                // EatsAtLeastPropagator::VisitLoopChoiceContinueNode.
                let (continue_node, loop_node, read_backward) = loop_parts(nodes, id);
                let following = nodes[continue_node.index()].info;
                nodes[id.index()].info.add_from_following(following);
                if !read_backward {
                    nodes[id.index()].eats = eats(nodes, continue_node);
                }
                // Check the loop last: it may need the value of this node.
                stack.push(Step::AfterLoopBody(id));
                stack.push(Step::Ensure(loop_node));
            }
            Step::AfterLoopBody(id) => {
                // AssertionPropagator::VisitLoopChoiceLoopNode; the eats-at-least
                // propagator does nothing here.
                let (_, loop_node, _) = loop_parts(nodes, id);
                let following = nodes[loop_node.index()].info;
                nodes[id.index()].info.add_from_following(following);
            }
            Step::AfterLookaround(id) => {
                visit_choice_info(nodes, id, LOOKAROUND_INDEX);
                let continue_node = alternative(nodes, id, CONTINUE_INDEX);
                stack.push(Step::AfterContinue(id));
                stack.push(Step::Ensure(continue_node));
            }
            Step::AfterContinue(id) => {
                visit_choice_info(nodes, id, CONTINUE_INDEX);
                let continue_node = alternative(nodes, id, CONTINUE_INDEX);
                nodes[id.index()].eats = eats(nodes, continue_node);
            }
            Step::Done(id) => {
                let info = &mut nodes[id.index()].info;
                info.being_analyzed = false;
                info.been_analyzed = true;
            }
        }
    }
}

fn alternative(nodes: &[Node], id: NodeId, i: usize) -> NodeId {
    match &nodes[id.index()].kind {
        NodeKind::Choice(c) => c.alternatives[i].node,
        _ => unreachable!("a choice"),
    }
}

fn loop_parts(nodes: &[Node], id: NodeId) -> (NodeId, NodeId, bool) {
    match &nodes[id.index()].kind {
        NodeKind::Choice(c) => match c.kind {
            ChoiceKind::Loop(l) => (
                l.continue_node.expect("a loop has a continue node"),
                l.loop_node.expect("a loop has a loop node"),
                l.read_backward,
            ),
            _ => unreachable!("a loop"),
        },
        _ => unreachable!("a loop"),
    }
}

/// The first part of V8's `Visit*`: the work before the first `EnsureAnalyzed` of a
/// successor, and that `EnsureAnalyzed`.
fn enter(nodes: &mut [Node], id: NodeId, one_byte: bool, stack: &mut Vec<Step>) {
    let node = &mut nodes[id.index()];
    match &node.kind {
        NodeKind::End(_) => {}
        NodeKind::Text(_) => {
            make_case_independent(node, one_byte);
            let next = node.on_success().expect("a successor");
            stack.push(Step::AfterSuccessor(id));
            stack.push(Step::Ensure(next));
        }
        NodeKind::Action(_)
        | NodeKind::UnanchoredAdvance { .. }
        | NodeKind::BackReference(_)
        | NodeKind::Assertion(_) => {
            let next = node.on_success().expect("a successor");
            stack.push(Step::AfterSuccessor(id));
            stack.push(Step::Ensure(next));
        }
        NodeKind::Choice(c) => match c.kind {
            ChoiceKind::Plain => {
                if let Some(first) = c.alternatives.first() {
                    stack.push(Step::AfterAlternative(id, 0));
                    stack.push(Step::Ensure(first.node));
                }
            }
            ChoiceKind::Loop(l) => {
                debug_assert_eq!(c.alternatives.len(), 2);
                // First the continuation, then the loop body.
                stack.push(Step::AfterLoopContinue(id));
                stack.push(Step::Ensure(l.continue_node.expect("a continue node")));
            }
            ChoiceKind::NegativeLookaround => {
                debug_assert_eq!(c.alternatives.len(), 2);
                stack.push(Step::AfterLookaround(id));
                stack.push(Step::Ensure(c.alternatives[LOOKAROUND_INDEX].node));
            }
        },
    }
}

/// The propagators of a sequence node, after its successor is analyzed.
fn after_successor(nodes: &mut [Node], id: NodeId) {
    let next = nodes[id.index()].on_success().expect("a successor");
    let next_info = nodes[next.index()].info;
    let next_eats = eats(nodes, next);
    let node = &mut nodes[id.index()];
    match &node.kind {
        NodeKind::Text(_) => {
            calculate_offsets(node);
            let NodeKind::Text(t) = &node.kind else {
                unreachable!()
            };
            // AssertionPropagator::VisitText does nothing. The eats-at-least value is not
            // used when reading backward. After the text the position is not at the start.
            if !t.read_backward {
                node.eats =
                    EatsAtLeastInfo::new(sat(t.length() + u32::from(next_eats.from_not_start)));
            }
        }
        NodeKind::Action(a) => {
            node.info.add_from_following(next_info);
            match a.action {
                Action::BeginPositiveSubmatch { success_node, .. } => {
                    // The body of a look-ahead eats nothing: take the node after it.
                    let NodeKind::Action(success) = &nodes[success_node.index()].kind else {
                        unreachable!("a submatch success action")
                    };
                    let after = eats(nodes, success.on_success);
                    nodes[id.index()].eats = after;
                }
                Action::PositiveSubmatchSuccess { .. } => {
                    // It rewinds the input: no value goes through it.
                    debug_assert!(node.eats.is_zero());
                }
                Action::EatsAtLeast { characters } => {
                    let mut e = next_eats;
                    e.set_max(u32::from(characters));
                    node.eats = e;
                }
                _ => node.eats = next_eats,
            }
        }
        NodeKind::UnanchoredAdvance { .. } => {
            node.info.add_from_following(next_info);
            node.eats = EatsAtLeastInfo::new(sat(1 + u32::from(next_eats.from_not_start)));
        }
        NodeKind::BackReference(b) => {
            if !b.read_backward {
                node.eats = next_eats;
            }
        }
        NodeKind::Assertion(a) => {
            let mut e = next_eats;
            if a.kind == AssertionKind::AtStart {
                // Not at the start, the node never succeeds; any answer is right, and the
                // largest does not stop the preload of the other branches.
                e.from_not_start = u8::MAX;
            }
            node.eats = e;
        }
        NodeKind::End(_) | NodeKind::Choice(_) => unreachable!("not a sequence node"),
    }
}

/// `AssertionPropagator::VisitChoice` for alternative `i`.
fn visit_choice_info(nodes: &mut [Node], id: NodeId, i: usize) {
    let alt = alternative(nodes, id, i);
    let following = nodes[alt.index()].info;
    nodes[id.index()].info.add_from_following(following);
}

/// Both propagators' `VisitChoice` for alternative `i` of a plain choice.
fn visit_choice(nodes: &mut [Node], id: NodeId, i: usize) {
    visit_choice_info(nodes, id, i);
    let alt = alternative(nodes, id, i);
    // The shortest match of a choice is the shortest of its alternatives.
    let mut e = if i == 0 {
        EatsAtLeastInfo::new(u8::MAX)
    } else {
        nodes[id.index()].eats
    };
    e.set_min(eats(nodes, alt));
    nodes[id.index()].eats = e;
}
