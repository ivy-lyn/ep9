// Copyright 2019 the V8 project authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// Ported for ep9 from V8 src/regexp/regexp-compiler.h and src/regexp/regexp-compiler.cc (the
// RegExpCompiler class: registers, flags, PreprocessRegExp; AnalyzeRegExp) and
// src/regexp/regexp.cc (the order of RegExpImpl::Compile) at
// 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0.
// The V8 licence text is in THIRD-PARTY-LICENSE (entry "V8 (BSD-3-Clause)").

//! The RX1 compiler (port step 3, docs/plan/04-execution.md section 3): the node graph and
//! its analysis. Step 4 (the bytecode and its writer) reads the graph and the analysis.
//!
//! ```text
//! Pattern (parser)  --lower-->  Re (ir.rs)  --node_of-->  node graph (node.rs)
//!   with CompileOptions { one_byte, optimize }        tonode.rs: V8 *::ToNode, PreprocessRegExp
//!                                                     (capture 0, the .*? search loop)
//!   --analyze--> eats-at-least, text offsets, the case closure of i (analysis.rs)
//! Program queries (V8 functions the code generator calls):
//!   quick_check(node, n)           QuickCheckDetails       quick_check.rs
//!   search_loop_lookahead(loop)    BoyerMooreLookahead     boyer_moore.rs
//!   first_character_set(node)      FirstCharacterSet       first_char.rs
//!   quick_check_filters()          ComputeQuickCheckFilters
//! ```
//!
//! A program is for one subject form: `one_byte` (Latin-1 subjects; the compiler drops what
//! such a subject cannot match) or two-byte (UTF-16), as V8 compiles one program per form.
//!
//! Named gaps of the port (none changes what a pattern matches):
//! - V8 checks the native stack while it makes nodes and in the analysis
//!   (`kAnalysisStackOverflow`); here node making recurses only over the tree (at most
//!   [`crate::MAX_NESTING`] groups deep) and the analysis keeps its own stack, so neither
//!   fails.
//! - V8 groups the surrogate pairs of a class by lead range in a hash map, so the order of
//!   those alternatives follows the hash; here it follows the lead range. The alternatives
//!   match disjoint code units, so the order does not change a match.
//! - The `min_match` of a class (see `ir.rs`).

pub(crate) mod analysis;
pub(crate) mod boyer_moore;
pub(crate) mod first_char;
pub(crate) mod ir;
pub mod node;
pub(crate) mod quick_check;
pub(crate) mod tonode;

use crate::error::Error;
use crate::flags::Flags;
use crate::parser::Pattern;

pub use boyer_moore::{BoyerMooreLookahead, FrequencyCollator, PositionInfo, SkipPlan};
pub use first_char::QuickCheckFilters;
pub use node::{
    Action, ActionNode, AssertionKind, AssertionNode, BackReferenceNode, ChoiceKind, ChoiceNode,
    ClassNode, EatsAtLeastInfo, End, Guard, GuardedAlternative, LoopInfo, Node, NodeId, NodeInfo,
    NodeKind, Relation, TextElement, TextElementKind, TextNode,
};
pub use quick_check::{QuickCheckDetails, QuickCheckPosition, preload_characters};

use node::{EatsAtLeastInfo as Eats, LARGE_EATS_AT_LEAST_VALUE};

/// V8 `RegExpMacroAssembler::kMaxRegister`.
pub const MAX_REGISTER: u32 = (1 << 16) - 1;
/// V8 `RegExp::kMaxOptimizedPatternLength`: longer sources compile without the unrolling.
pub const MAX_OPTIMIZED_PATTERN_LENGTH: usize = 20 * 1024;
/// V8 `Node::kRecursionBudget`.
pub const RECURSION_BUDGET: i32 = 200;
/// V8 `kMaxLookaheadForBoyerMoore`.
pub const MAX_LOOKAHEAD_FOR_BOYER_MOORE: u32 = 8;

/// How to compile (V8's `is_one_byte` and `RegExpCompiler::optimize`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompileOptions {
    /// The program is for Latin-1 subjects.
    pub one_byte: bool,
    /// V8 `optimize`: `--regexp-optimization` and a source of at most
    /// [`MAX_OPTIMIZED_PATTERN_LENGTH`] code units ([`CompileOptions::for_source`]).
    pub optimize: bool,
}

impl CompileOptions {
    /// The options V8 uses for a source of `source_length` code units.
    #[must_use]
    pub const fn for_source(one_byte: bool, source_length: usize) -> CompileOptions {
        CompileOptions {
            one_byte,
            optimize: source_length <= MAX_OPTIMIZED_PATTERN_LENGTH,
        }
    }
}

/// A compiled pattern: the node graph after the analysis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub(crate) nodes: Vec<Node>,
    start: NodeId,
    accept: NodeId,
    register_count: u32,
    capture_count: u32,
    flags: Flags,
    one_byte: bool,
    has_search_prefix: bool,
    backreferenced: Vec<bool>,
}

impl Program {
    /// The node the match starts at.
    #[must_use]
    pub const fn start(&self) -> NodeId {
        self.start
    }

    /// The accept node.
    #[must_use]
    pub const fn accept(&self) -> NodeId {
        self.accept
    }

    /// A node.
    #[must_use]
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    /// Every node, by index (some are not reachable from [`Program::start`], as in V8).
    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// The registers the program uses (V8 `next_register_`): two per capture, capture 0
    /// first, then the loop counters and look-around registers.
    #[must_use]
    pub const fn register_count(&self) -> u32 {
        self.register_count
    }

    /// The number of capturing groups (capture 0 not counted).
    #[must_use]
    pub const fn capture_count(&self) -> u32 {
        self.capture_count
    }

    /// The flags of the pattern.
    #[must_use]
    pub const fn flags(&self) -> Flags {
        self.flags
    }

    /// The program is for Latin-1 subjects.
    #[must_use]
    pub const fn one_byte(&self) -> bool {
        self.one_byte
    }

    /// The compiler put the `.*?` search loop before the pattern (V8 `has_search_prefix`).
    #[must_use]
    pub const fn has_search_prefix(&self) -> bool {
        self.has_search_prefix
    }

    /// V8 `ActionNode::stores_backreferenced_capture`.
    #[must_use]
    pub fn stores_backreferenced_capture(&self, action: Action) -> bool {
        match action {
            Action::StorePosition {
                capture: Some(c), ..
            } => self
                .backreferenced
                .get(c as usize)
                .copied()
                .unwrap_or(false),
            _ => false,
        }
    }

    /// V8 `Node::EatsAtLeast`.
    #[must_use]
    pub fn eats_at_least(&self, id: NodeId, not_at_start: bool) -> u32 {
        self.node(id).eats_at_least(not_at_start)
    }

    /// The nodes reachable from the start, in the order a depth-first walk first meets them.
    #[must_use]
    pub fn reachable(&self) -> Vec<NodeId> {
        let mut seen = vec![false; self.nodes.len()];
        let mut order = Vec::new();
        let mut stack = vec![self.start];
        while let Some(id) = stack.pop() {
            if std::mem::replace(&mut seen[id.index()], true) {
                continue;
            }
            order.push(id);
            let node = self.node(id);
            if let NodeKind::Choice(c) = &node.kind {
                stack.extend(c.alternatives.iter().rev().map(|a| a.node));
            } else if let Some(next) = node.on_success() {
                stack.push(next);
            }
            if let NodeKind::Action(ActionNode {
                action: Action::BeginPositiveSubmatch { success_node, .. },
                ..
            }) = node.kind
            {
                stack.push(success_node);
            }
        }
        order
    }

    /// Checks the facts the code generator relies on, for each node reachable from the
    /// start: the analysis visited it, its links and registers are in range, the parts of
    /// loops and look-arounds have their form, the text offsets add up, and the
    /// eats-at-least value away from the start is at least the one at the start.
    ///
    /// # Errors
    ///
    /// The first fact that does not hold, as text.
    pub fn validate(&self) -> Result<(), String> {
        let in_range = |id: NodeId| id.index() < self.nodes.len();
        let reg = |r: u32, what: &str| {
            if r < self.register_count {
                Ok(())
            } else {
                Err(format!("{what} register {r} >= {}", self.register_count))
            }
        };
        for id in self.reachable() {
            let node = self.node(id);
            let at = |e: String| format!("node {}: {e}", id.index());
            if !node.info.been_analyzed || node.info.being_analyzed {
                return Err(at("not analyzed".into()));
            }
            if node.eats.from_not_start < node.eats.from_possibly_start {
                return Err(at(format!("eats {:?}", node.eats)));
            }
            if let Some(next) = node.on_success()
                && !in_range(next)
            {
                return Err(at("successor out of range".into()));
            }
            match &node.kind {
                NodeKind::End(End::NegativeSubmatchSuccess {
                    stack_pointer_register,
                    position_register,
                    ..
                }) => {
                    reg(*stack_pointer_register, "stack").map_err(at)?;
                    reg(*position_register, "position").map_err(at)?;
                }
                NodeKind::End(_) | NodeKind::UnanchoredAdvance { .. } => {}
                NodeKind::Action(a) => match a.action {
                    Action::SetRegisterForLoop { register, .. }
                    | Action::IncrementRegister { register }
                    | Action::StorePosition { register, .. }
                    | Action::RestorePosition { register } => {
                        reg(register, "action").map_err(at)?
                    }
                    Action::ClearCaptures { from, to } => {
                        if from > to {
                            return Err(at("an empty clear".into()));
                        }
                        reg(to, "clear").map_err(at)?;
                    }
                    Action::EmptyMatchCheck {
                        start_register,
                        repetition_register,
                        ..
                    } => {
                        reg(start_register, "empty check").map_err(at)?;
                        if let Some(r) = repetition_register {
                            reg(r, "counter").map_err(at)?;
                        }
                    }
                    Action::BeginPositiveSubmatch {
                        stack_pointer_register,
                        position_register,
                        success_node,
                    } => {
                        let ok = matches!(
                            self.node(success_node).kind,
                            NodeKind::Action(ActionNode {
                                action: Action::PositiveSubmatchSuccess {
                                    stack_pointer_register: s,
                                    position_register: p,
                                    ..
                                },
                                ..
                            }) if s == stack_pointer_register && p == position_register
                        );
                        if !ok {
                            return Err(at("a positive submatch without its success".into()));
                        }
                    }
                    Action::BeginNegativeSubmatch { .. } => {
                        let ok = matches!(
                            &self.node(a.on_success).kind,
                            NodeKind::Choice(ChoiceNode {
                                kind: ChoiceKind::NegativeLookaround,
                                ..
                            })
                        );
                        if !ok {
                            return Err(at("a negative submatch without its choice".into()));
                        }
                    }
                    Action::PositiveSubmatchSuccess { .. } | Action::EatsAtLeast { .. } => {}
                },
                NodeKind::Text(t) => {
                    if t.elements.is_empty() {
                        return Err(at("an empty text node".into()));
                    }
                    let mut offset = 0;
                    for e in &t.elements {
                        if e.cp_offset != offset {
                            return Err(at("text offsets".into()));
                        }
                        offset += e.kind.length();
                    }
                }
                NodeKind::Assertion(_) => {}
                NodeKind::BackReference(b) => {
                    if b.end_register != b.start_register + 1
                        || b.end_register >= 2 * (self.capture_count + 1)
                    {
                        return Err(at("back reference registers".into()));
                    }
                }
                NodeKind::Choice(c) => {
                    if c.alternatives.iter().any(|a| !in_range(a.node)) {
                        return Err(at("alternative out of range".into()));
                    }
                    for a in &c.alternatives {
                        if let Some(g) = a.guard {
                            reg(g.register, "guard").map_err(at)?;
                        }
                    }
                    match c.kind {
                        ChoiceKind::Plain => {
                            if c.alternatives.len() < 2 {
                                return Err(at("a choice of fewer than 2".into()));
                            }
                        }
                        ChoiceKind::Loop(l) => {
                            let nodes: Vec<NodeId> =
                                c.alternatives.iter().map(|a| a.node).collect();
                            let ok = nodes.len() == 2
                                && l.loop_node.is_some_and(|n| nodes.contains(&n))
                                && l.continue_node.is_some_and(|n| nodes.contains(&n));
                            if !ok {
                                return Err(at("a loop without its two parts".into()));
                            }
                        }
                        ChoiceKind::NegativeLookaround => {
                            if c.alternatives.len() != 2 {
                                return Err(at("a negative look-around of not 2".into()));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Compiles a parsed pattern (V8 `RegExpImpl::Compile` up to the code generator:
/// `PreprocessRegExp`, then `AnalyzeRegExp`).
///
/// # Errors
///
/// [`Error::TooLarge`] when the pattern needs more than [`MAX_REGISTER`] registers.
pub fn compile(pattern: &Pattern, flags: Flags, options: CompileOptions) -> Result<Program, Error> {
    let mut compiler = Compiler::new(pattern.capture_count, flags, options);
    let tree = ir::lower(&pattern.tree, flags);
    let start = compiler.preprocess_regexp(&tree, pattern.contains_anchor);
    if compiler.too_big {
        return Err(Error::TooLarge);
    }
    analysis::analyze(&mut compiler.nodes, start, options.one_byte);
    Ok(Program {
        nodes: compiler.nodes,
        start,
        accept: compiler.accept,
        register_count: compiler.next_register,
        capture_count: pattern.capture_count,
        flags,
        one_byte: options.one_byte,
        has_search_prefix: compiler.has_search_prefix,
        backreferenced: compiler.backreferenced,
    })
}

/// V8 `RegExpCompiler` (the part that makes nodes).
pub(crate) struct Compiler {
    pub(crate) nodes: Vec<Node>,
    pub(crate) accept: NodeId,
    pub(crate) next_register: u32,
    unicode_lookaround_stack_register: Option<u32>,
    unicode_lookaround_position_register: Option<u32>,
    pub(crate) flags: Flags,
    pub(crate) one_byte: bool,
    pub(crate) too_big: bool,
    pub(crate) optimize: bool,
    pub(crate) read_backward: bool,
    pub(crate) current_expansion_factor: u32,
    pub(crate) has_search_prefix: bool,
    /// V8 `Capture::is_backreferenced`, by capture index.
    pub(crate) backreferenced: Vec<bool>,
}

impl Compiler {
    fn new(capture_count: u32, flags: Flags, options: CompileOptions) -> Compiler {
        let mut c = Compiler {
            nodes: Vec::new(),
            accept: NodeId(0),
            // V8 JSRegExp::RegistersForCaptureCount.
            next_register: (capture_count + 1) * 2,
            unicode_lookaround_stack_register: None,
            unicode_lookaround_position_register: None,
            flags,
            one_byte: options.one_byte,
            too_big: false,
            optimize: options.optimize,
            read_backward: false,
            current_expansion_factor: 1,
            has_search_prefix: false,
            backreferenced: vec![false; capture_count as usize + 1],
        };
        c.accept = c.add(NodeKind::End(End::Accept));
        c
    }

    /// Adds a node with the flags in force (V8 `zone->New<..>(.., compiler->flags())`).
    pub(crate) fn add(&mut self, kind: NodeKind) -> NodeId {
        let eats = if matches!(kind, NodeKind::End(End::Backtrack)) {
            Eats::new(LARGE_EATS_AT_LEAST_VALUE)
        } else {
            Eats::default()
        };
        let id = NodeId(u32::try_from(self.nodes.len()).expect("fewer than 2^32 nodes"));
        self.nodes.push(Node {
            kind,
            flags: self.flags,
            info: NodeInfo::default(),
            eats,
        });
        id
    }

    /// A new backtrack node (V8 `New<EndNode>(EndNode::BACKTRACK, ..)`).
    pub(crate) fn backtrack(&mut self) -> NodeId {
        self.add(NodeKind::End(End::Backtrack))
    }

    pub(crate) fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    pub(crate) fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.index()]
    }

    /// V8 `AllocateRegister`.
    pub(crate) fn allocate_register(&mut self) -> u32 {
        if self.next_register >= MAX_REGISTER {
            self.too_big = true;
            return self.next_register;
        }
        let r = self.next_register;
        self.next_register += 1;
        r
    }

    /// V8 `UnicodeLookaroundStackRegister`.
    pub(crate) fn unicode_lookaround_stack_register(&mut self) -> u32 {
        if let Some(r) = self.unicode_lookaround_stack_register {
            return r;
        }
        let r = self.allocate_register();
        self.unicode_lookaround_stack_register = Some(r);
        r
    }

    /// V8 `UnicodeLookaroundPositionRegister`.
    pub(crate) fn unicode_lookaround_position_register(&mut self) -> u32 {
        if let Some(r) = self.unicode_lookaround_position_register {
            return r;
        }
        let r = self.allocate_register();
        self.unicode_lookaround_position_register = Some(r);
        r
    }
}
