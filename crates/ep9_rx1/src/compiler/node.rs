// Copyright 2019 the V8 project authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// Ported for ep9 from V8 src/regexp/regexp-nodes.h (the node kinds, NodeInfo,
// EatsAtLeastInfo, Guard, GuardedAlternative) and src/regexp/regexp-ast.h (TextElement) at
// 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0.
// The V8 licence text is in THIRD-PARTY-LICENSE (entry "V8 (BSD-3-Clause)").

//! The node graph (V8 `regexp-nodes.h`).
//!
//! ```text
//! V8 (zone objects, virtual methods)          RX1 (one arena, NodeId indices)
//! EndNode (ACCEPT, BACKTRACK)                 NodeKind::End(End::Accept | End::Backtrack)
//! NegativeSubmatchSuccess                     NodeKind::End(End::NegativeSubmatchSuccess)
//! ActionNode (10 action types)                NodeKind::Action(ActionNode { action: Action })
//! TextNode, AssertionNode, BackReferenceNode  NodeKind::Text, Assertion, BackReference
//! UnanchoredAdvanceNode                       NodeKind::UnanchoredAdvance
//! ChoiceNode, LoopChoiceNode,                 NodeKind::Choice(ChoiceNode { kind: Plain |
//!   NegativeLookaroundChoiceNode                Loop(..) | NegativeLookaround })
//! ```
//!
//! The graph has cycles (a loop returns to its `LoopChoiceNode`), so the nodes live in one
//! vector and point at each other by [`NodeId`]. A node never moves and is never removed.

use crate::ast::StandardSet;
use crate::charset::CharacterRange;
use crate::flags::Flags;

/// The index of a node in its [`super::Program`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub(crate) u32);

impl NodeId {
    /// The index in the node list.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// V8 `NodeInfo`: the analysis state and the assertion interests of a node.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NodeInfo {
    pub(crate) being_analyzed: bool,
    pub(crate) been_analyzed: bool,
    /// The node has to know if the preceding character was a word character.
    pub follows_word_interest: bool,
    /// The node has to know if the preceding character was a newline.
    pub follows_newline_interest: bool,
    /// The node has to know if it is at the start of the input.
    pub follows_start_interest: bool,
    /// V8 `at_end`.
    pub at_end: bool,
}

impl NodeInfo {
    /// V8 `AddFromFollowing`.
    pub(crate) fn add_from_following(&mut self, that: NodeInfo) {
        self.follows_word_interest |= that.follows_word_interest;
        self.follows_newline_interest |= that.follows_newline_interest;
        self.follows_start_interest |= that.follows_start_interest;
    }
}

/// V8 `EatsAtLeastInfo`: any successful match from the node consumes at least this many
/// characters (saturated at 255).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EatsAtLeastInfo {
    /// When the node may be at the start of the input.
    pub from_possibly_start: u8,
    /// When the node is known not to be at the start (`^` cannot match): at least
    /// `from_possibly_start`.
    pub from_not_start: u8,
}

impl EatsAtLeastInfo {
    /// Both values `eats`.
    #[must_use]
    pub const fn new(eats: u8) -> EatsAtLeastInfo {
        EatsAtLeastInfo {
            from_possibly_start: eats,
            from_not_start: eats,
        }
    }

    /// V8 `SetMin`.
    pub(crate) fn set_min(&mut self, other: EatsAtLeastInfo) {
        self.from_possibly_start = self.from_possibly_start.min(other.from_possibly_start);
        self.from_not_start = self.from_not_start.min(other.from_not_start);
    }

    /// V8 `SetMax` (the value saturated to `u8`).
    pub(crate) fn set_max(&mut self, other: u32) {
        let max = u8::try_from(other).unwrap_or(u8::MAX);
        self.from_possibly_start = self.from_possibly_start.max(max);
        self.from_not_start = self.from_not_start.max(max);
    }

    /// V8 `IsZero`.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.from_possibly_start == 0 && self.from_not_start == 0
    }
}

/// V8 `Node::kLargeEatsAtLeastValue`: the eats-at-least value of a backtrack node.
pub const LARGE_EATS_AT_LEAST_VALUE: u8 = 255;

/// One node of the graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// What the node does.
    pub kind: NodeKind,
    /// The flags in force where the node was made (modifier groups change them).
    pub flags: Flags,
    pub(crate) info: NodeInfo,
    pub(crate) eats: EatsAtLeastInfo,
}

impl Node {
    /// V8 `Node::info`.
    #[must_use]
    pub const fn info(&self) -> NodeInfo {
        self.info
    }

    /// V8 `eats_at_least_info` (zero until the analysis ran, except a backtrack node).
    #[must_use]
    pub const fn eats_at_least_info(&self) -> EatsAtLeastInfo {
        self.eats
    }

    /// V8 `Node::EatsAtLeast`.
    #[must_use]
    pub fn eats_at_least(&self, not_at_start: bool) -> u32 {
        u32::from(if not_at_start {
            self.eats.from_not_start
        } else {
            self.eats.from_possibly_start
        })
    }

    /// V8 `IsBacktrack`.
    #[must_use]
    pub const fn is_backtrack(&self) -> bool {
        matches!(self.kind, NodeKind::End(End::Backtrack))
    }

    /// The successor of a node with one successor (V8 `SeqNode::on_success`).
    #[must_use]
    pub const fn on_success(&self) -> Option<NodeId> {
        match &self.kind {
            NodeKind::Action(a) => Some(a.on_success),
            NodeKind::Text(t) => Some(t.on_success),
            NodeKind::Assertion(a) => Some(a.on_success),
            NodeKind::BackReference(b) => Some(b.on_success),
            NodeKind::UnanchoredAdvance { on_success } => Some(*on_success),
            NodeKind::End(_) | NodeKind::Choice(_) => None,
        }
    }
}

/// The node kinds (V8 `FOR_EACH_NODE_TYPE`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// V8 `EndNode` and `NegativeSubmatchSuccess`.
    End(End),
    /// V8 `ActionNode`.
    Action(ActionNode),
    /// V8 `TextNode`.
    Text(TextNode),
    /// V8 `AssertionNode`.
    Assertion(AssertionNode),
    /// V8 `BackReferenceNode`.
    BackReference(BackReferenceNode),
    /// V8 `UnanchoredAdvanceNode`: one code point forward (ES AdvanceStringIndex).
    UnanchoredAdvance {
        /// The next node.
        on_success: NodeId,
    },
    /// V8 `ChoiceNode` and its subclasses.
    Choice(ChoiceNode),
}

/// The end nodes (V8 `EndNode::Action`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// The match succeeds.
    Accept,
    /// The match fails here.
    Backtrack,
    /// The body of a negative look-around matched (V8 `NegativeSubmatchSuccess`): restore
    /// the position and the backtrack stack, clear the captures of the body, backtrack.
    NegativeSubmatchSuccess {
        /// The register that holds the backtrack stack pointer.
        stack_pointer_register: u32,
        /// The register that holds the position.
        position_register: u32,
        /// The capture registers of the body: count.
        clear_capture_count: u32,
        /// The capture registers of the body: the first one.
        clear_capture_start: u32,
    },
}

/// V8 `ActionNode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionNode {
    /// The action.
    pub action: Action,
    /// The next node.
    pub on_success: NodeId,
}

/// V8 `ActionNode::ActionType` with its data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// `reg = value` (a loop counter).
    SetRegisterForLoop {
        /// The register.
        register: u32,
        /// The value.
        value: u32,
    },
    /// `reg += 1`.
    IncrementRegister {
        /// The register.
        register: u32,
    },
    /// `reg = position`.
    StorePosition {
        /// The register.
        register: u32,
        /// The capture whose start or end this is (`None` for the implicit capture 0).
        capture: Option<u32>,
    },
    /// `position = reg`.
    RestorePosition {
        /// The register.
        register: u32,
    },
    /// The start of a positive look-around: save the position and the backtrack stack.
    BeginPositiveSubmatch {
        /// The register for the backtrack stack pointer.
        stack_pointer_register: u32,
        /// The register for the position.
        position_register: u32,
        /// The `PositiveSubmatchSuccess` node at the end of the body; its successor is the
        /// node after the look-around.
        success_node: NodeId,
    },
    /// The start of a negative look-around; the successor is a negative look-around
    /// choice.
    BeginNegativeSubmatch {
        /// The register for the backtrack stack pointer.
        stack_pointer_register: u32,
        /// The register for the position.
        position_register: u32,
    },
    /// The body of a positive look-around matched: restore the position and the backtrack
    /// stack; the captures of the body are cleared when the rest fails.
    PositiveSubmatchSuccess {
        /// The register that holds the backtrack stack pointer.
        stack_pointer_register: u32,
        /// The register that holds the position.
        position_register: u32,
        /// The capture registers of the body: count.
        clear_register_count: u32,
        /// The capture registers of the body: the first one.
        clear_register_from: u32,
    },
    /// The loop body matched the empty string: backtrack (unless the minimum count is not
    /// reached yet).
    EmptyMatchCheck {
        /// The register with the position at the start of the body.
        start_register: u32,
        /// The loop counter, if the loop has a minimum.
        repetition_register: Option<u32>,
        /// The minimum count.
        repetition_limit: u32,
    },
    /// Clears the registers `from..=to` (the captures of a loop body).
    ClearCaptures {
        /// The first register.
        from: u32,
        /// The last register.
        to: u32,
    },
    /// A marker for the analysis: the rest eats at least `characters`.
    EatsAtLeast {
        /// The count.
        characters: u8,
    },
}

impl Action {
    /// V8 `IsSimpleAction`.
    #[must_use]
    pub const fn is_simple(self) -> bool {
        matches!(
            self,
            Action::StorePosition { .. }
                | Action::RestorePosition { .. }
                | Action::IncrementRegister { .. }
                | Action::SetRegisterForLoop { .. }
                | Action::ClearCaptures { .. }
        )
    }
}

/// V8 `TextNode`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextNode {
    /// The elements, in pattern order.
    pub elements: Vec<TextElement>,
    /// The node reads to the left (look-behind).
    pub read_backward: bool,
    /// The next node.
    pub on_success: NodeId,
}

impl TextNode {
    /// V8 `TextNode::Length` (the offsets are set by the analysis).
    #[must_use]
    pub fn length(&self) -> u32 {
        self.elements
            .last()
            .map_or(0, |e| e.cp_offset + e.kind.length())
    }
}

/// V8 `TextElement`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextElement {
    /// The offset of the element in its node, in code units (set by the analysis).
    pub cp_offset: u32,
    /// The element.
    pub kind: TextElementKind,
}

/// The two kinds of text element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextElementKind {
    /// Code units (case independent with `i`).
    Atom(Vec<u16>),
    /// One code unit of a class.
    Class(ClassNode),
}

impl TextElementKind {
    /// V8 `TextElement::length`.
    #[must_use]
    pub fn length(&self) -> u32 {
        match self {
            TextElementKind::Atom(units) => units.len() as u32,
            TextElementKind::Class(_) => 1,
        }
    }
}

/// A class in a text node (V8 `ClassRanges` as the code generator sees it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassNode {
    /// The ranges (canonical when made; the analysis may append the case equivalents of
    /// `i` without `u`, as V8 does).
    pub ranges: Vec<CharacterRange>,
    /// The class matches what is not in `ranges`.
    pub negated: bool,
    /// The class is a standard set (V8 `CharacterSet::standard_set_type`; only the classes
    /// the compiler makes, as V8's parser makes every class from ranges).
    pub standard: Option<StandardSet>,
    /// V8 `CONTAINS_SPLIT_SURROGATE`.
    pub contains_split_surrogate: bool,
}

/// V8 `AssertionNode::AssertionType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssertionKind {
    /// At the end of the input.
    AtEnd,
    /// At the start of the input.
    AtStart,
    /// `\b`.
    AtBoundary,
    /// `\B`.
    AtNonBoundary,
    /// At the start of the input or after a line terminator (`^` with `m`).
    AfterNewline,
}

/// V8 `AssertionNode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssertionNode {
    /// The assertion.
    pub kind: AssertionKind,
    /// The next node.
    pub on_success: NodeId,
}

/// V8 `BackReferenceNode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackReferenceNode {
    /// The capture start register.
    pub start_register: u32,
    /// The capture end register.
    pub end_register: u32,
    /// The node reads to the left.
    pub read_backward: bool,
    /// The next node.
    pub on_success: NodeId,
}

/// V8 `Guard::Relation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Relation {
    /// `register < value`.
    Lt,
    /// `register >= value`.
    Geq,
}

/// V8 `Guard`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Guard {
    /// The register.
    pub register: u32,
    /// The test.
    pub relation: Relation,
    /// The value.
    pub value: u32,
}

/// V8 `GuardedAlternative` (V8 keeps a list of guards; it never has more than one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuardedAlternative {
    /// The first node of the alternative.
    pub node: NodeId,
    /// The test that must hold to take it.
    pub guard: Option<Guard>,
}

impl GuardedAlternative {
    /// An alternative with no guard.
    #[must_use]
    pub const fn new(node: NodeId) -> GuardedAlternative {
        GuardedAlternative { node, guard: None }
    }
}

/// V8 `ChoiceNode`, `LoopChoiceNode` and `NegativeLookaroundChoiceNode`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceNode {
    /// The alternatives, tried in order.
    pub alternatives: Vec<GuardedAlternative>,
    /// The node is never at the start of the input (V8 `not_at_start_`).
    pub not_at_start: bool,
    /// V8 `SetDoNotInline` (a class with more than 32 ranges).
    pub do_not_inline: bool,
    /// The subclass.
    pub kind: ChoiceKind,
}

/// The subclass of a [`ChoiceNode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceKind {
    /// V8 `ChoiceNode`.
    Plain,
    /// V8 `LoopChoiceNode`.
    Loop(LoopInfo),
    /// V8 `NegativeLookaroundChoiceNode`: alternative 0 is the body that must fail,
    /// alternative 1 the rest of the pattern.
    NegativeLookaround,
}

/// The data of a V8 `LoopChoiceNode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopInfo {
    /// The alternative that runs the body once more.
    pub loop_node: Option<NodeId>,
    /// The alternative that leaves the loop.
    pub continue_node: Option<NodeId>,
    /// The body can match the empty string.
    pub body_can_be_zero_length: bool,
    /// The loop is in a look-behind.
    pub read_backward: bool,
}

/// V8 `NegativeLookaroundChoiceNode::kLookaroundIndex`.
pub const LOOKAROUND_INDEX: usize = 0;
/// V8 `NegativeLookaroundChoiceNode::kContinueIndex`.
pub const CONTINUE_INDEX: usize = 1;

impl ChoiceNode {
    /// V8 `ChoiceNode::read_backward` (true only for a loop in a look-behind).
    #[must_use]
    pub const fn read_backward(&self) -> bool {
        match self.kind {
            ChoiceKind::Loop(l) => l.read_backward,
            _ => false,
        }
    }

    /// V8 `try_to_emit_quick_check_for_alternative`: a negative look-around skips the quick
    /// check of its body.
    #[must_use]
    pub const fn try_to_emit_quick_check_for_alternative(&self, is_first: bool) -> bool {
        match self.kind {
            ChoiceKind::NegativeLookaround => !is_first,
            _ => true,
        }
    }
}
