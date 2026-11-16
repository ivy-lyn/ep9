// Copyright 2016 the V8 project authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// Ported for ep9 from V8 src/regexp/regexp-ast.h and src/regexp/regexp-ast.cc (min_match,
// max_match, CaptureRegisters, IsCertainlyAnchoredAtStart/End, the ClassSetOperand and
// ClassSetExpression constructors, the ClassRanges flags) at
// 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0.
// The V8 licence text is in THIRD-PARTY-LICENSE (entry "V8 (BSD-3-Clause)").

//! The tree the compiler works on: the parser's [`Tree`] with the class items made into code
//! point ranges (step 2) and the facts V8 keeps on its tree nodes.
//!
//! ```text
//! Tree (ast.rs, owned, read only)        Re (this file)
//! ClassRanges { items, negated }         Re::Class(ClassRe): ranges + V8's ClassRanges flags
//! ClassSetOperand / ClassSetExpression   Re::ClassSet { min, max, body }: V8's
//!                                          ClassSetOperand::ToNodeImpl tree, made here
//! the rest                               one Re variant each
//! ```
//!
//! V8 changes its tree while it makes nodes (`Disjunction::SortConsecutiveAtoms`, the class
//! canonicalization, `ComputeExpression`). Here the parser's tree stays as it is; the
//! compiler works on this copy, and the disjunction rewrites make new lists
//! (`tonode.rs`). Each rewrite gives the same result when it runs again, so the copies that
//! V8 shares and RX1 does not (an unrolled quantifier body) give the same nodes.
//!
//! One named gap: V8 knows if a class came from brackets or from an escape (`\p{..}`
//! outside brackets has no `NO_CASE_FOLDING_NEEDED` flag, so its `min_match` is 1); the
//! parser's tree does not keep that. Here every class counts as bracketed: a class whose code
//! points are all above U+FFFF has `min_match` 2 without `i`. That only raises an
//! eats-at-least value to one that is still true (such a class always eats a surrogate
//! pair).

use std::sync::Arc;

use crate::ast::{
    AssertionType, ClassSetExpression, LookaroundType, QuantifierType, StandardSet, Tree,
};
use crate::charset::{self, CharacterRange};
use crate::flags::Flags;
use crate::sets;

/// V8 `Tree::kInfinity`.
pub(crate) const INFINITY: u32 = crate::ast::INFINITY;

/// A class of the compile-time tree (V8 `ClassRanges` with its flags).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClassRe {
    /// Canonical ranges, never empty (V8's constructor makes `[]` the negated everything).
    pub(crate) ranges: Vec<CharacterRange>,
    pub(crate) negated: bool,
    /// V8 `CharacterSet::standard_set_type` (only for the classes the compiler makes).
    pub(crate) standard: Option<StandardSet>,
    /// V8 `NO_CASE_FOLDING_NEEDED`.
    pub(crate) no_case_folding_needed: bool,
    /// V8 `CONTAINS_SPLIT_SURROGATE`.
    pub(crate) contains_split_surrogate: bool,
    /// V8 `IS_CERTAINLY_ONE_CODE_POINT`.
    pub(crate) one_code_point: bool,
    /// V8 `IS_CERTAINLY_TWO_CODE_POINTS`.
    pub(crate) two_code_points: bool,
}

impl ClassRe {
    /// V8 `ClassRanges(zone, ranges, flags)`: the empty list becomes the negated
    /// everything; a class of only supplementary code points is two code units long.
    pub(crate) fn new(
        mut ranges: Vec<CharacterRange>,
        mut negated: bool,
        no_case_folding_needed: bool,
        contains_split_surrogate: bool,
        one_code_point: bool,
    ) -> ClassRe {
        if ranges.is_empty() {
            ranges.push(CharacterRange::everything());
            negated = !negated;
        }
        charset::canonicalize(&mut ranges);
        let two_code_points =
            !negated && no_case_folding_needed && ranges.iter().all(|r| r.from() >= 0x10000);
        ClassRe {
            ranges,
            negated,
            standard: None,
            no_case_folding_needed,
            contains_split_surrogate,
            one_code_point,
            two_code_points,
        }
    }

    /// V8 `ClassRanges(StandardCharacterSet)` (no flags).
    pub(crate) fn standard(set: StandardSet) -> ClassRe {
        let mut ranges = Vec::new();
        charset::add_class_escape(set, &mut ranges, false);
        charset::canonicalize(&mut ranges);
        ClassRe {
            ranges,
            negated: false,
            standard: Some(set),
            no_case_folding_needed: false,
            contains_split_surrogate: false,
            one_code_point: false,
            two_code_points: false,
        }
    }

    fn min_match(&self) -> u32 {
        if self.two_code_points { 2 } else { 1 }
    }

    fn max_match(&self) -> u32 {
        if self.one_code_point { 1 } else { 2 }
    }
}

/// A text element of the compile-time tree (V8 `TextElement`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TextRe {
    Atom(Vec<u16>),
    Class(ClassRe),
}

impl TextRe {
    fn length(&self) -> u32 {
        match self {
            TextRe::Atom(units) => units.len() as u32,
            TextRe::Class(_) => 1,
        }
    }
}

/// A look-around of the compile-time tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LookaroundRe {
    pub(crate) positive: bool,
    pub(crate) lookbehind: bool,
    pub(crate) capture_count: u32,
    pub(crate) capture_from: u32,
    pub(crate) body: Box<Re>,
}

/// The compile-time tree (V8's `Tree` subclasses).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Re {
    Disjunction(Vec<Re>),
    Alternative(Vec<Re>),
    Assertion(AssertionType),
    Class(ClassRe),
    Atom(Vec<u16>),
    Text(Vec<TextRe>),
    Quantifier {
        min: u32,
        max: u32,
        greedy: bool,
        body: Box<Re>,
    },
    Capture {
        index: u32,
        body: Box<Re>,
    },
    Group {
        flags: Flags,
        body: Box<Re>,
    },
    Lookaround(LookaroundRe),
    BackReference(Arc<[u32]>),
    /// A `v`-mode class: V8's `ClassSetOperand`/`ClassSetExpression` with its own
    /// `min_match` and `max_match`, and the tree `ClassSetOperand::ToNodeImpl` makes
    /// (`None`: the empty class, a backtrack).
    ClassSet {
        min: u32,
        max: u32,
        body: Option<Box<Re>>,
    },
    Empty,
}

/// V8 `IncreaseBy` (saturates at [`INFINITY`]).
fn increase_by(previous: u32, increase: u32) -> u32 {
    if INFINITY - previous < increase {
        INFINITY
    } else {
        previous + increase
    }
}

/// V8's `min * body` of the `Quantifier` constructor (saturates at [`INFINITY`]).
fn times(count: u32, body: u32) -> u32 {
    if count > 0 && body > INFINITY / count {
        INFINITY
    } else {
        count * body
    }
}

impl Re {
    /// V8 `min_match`.
    pub(crate) fn min_match(&self) -> u32 {
        match self {
            Re::Disjunction(alts) => alts.iter().map(Re::min_match).min().unwrap_or(0),
            Re::Alternative(nodes) => nodes
                .iter()
                .fold(0, |acc, n| increase_by(acc, n.min_match())),
            Re::Assertion(_) | Re::Lookaround(_) | Re::BackReference(_) | Re::Empty => 0,
            Re::Class(c) => c.min_match(),
            Re::Atom(units) => units.len() as u32,
            Re::Text(elements) => elements.iter().map(TextRe::length).sum(),
            Re::Quantifier { min, body, .. } => times(*min, body.min_match()),
            Re::Capture { body, .. } | Re::Group { body, .. } => body.min_match(),
            Re::ClassSet { min, .. } => *min,
        }
    }

    /// V8 `max_match`.
    pub(crate) fn max_match(&self) -> u32 {
        match self {
            Re::Disjunction(alts) => alts.iter().map(Re::max_match).max().unwrap_or(0),
            Re::Alternative(nodes) => nodes
                .iter()
                .fold(0, |acc, n| increase_by(acc, n.max_match())),
            Re::Assertion(_) | Re::Lookaround(_) | Re::Empty => 0,
            Re::BackReference(_) => INFINITY,
            Re::Class(c) => c.max_match(),
            Re::Atom(units) => units.len() as u32,
            Re::Text(elements) => elements.iter().map(TextRe::length).sum(),
            Re::Quantifier { max, body, .. } => times(*max, body.max_match()),
            Re::Capture { body, .. } | Re::Group { body, .. } => body.max_match(),
            Re::ClassSet { max, .. } => *max,
        }
    }

    /// V8 `CaptureRegisters`: the capture registers inside, as `(from, to)`, or `None`
    /// (V8 `Interval::Empty`).
    pub(crate) fn capture_registers(&self) -> Option<(u32, u32)> {
        fn union(a: Option<(u32, u32)>, b: Option<(u32, u32)>) -> Option<(u32, u32)> {
            match (a, b) {
                (None, x) | (x, None) => x,
                (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.max(b.1))),
            }
        }
        match self {
            Re::Disjunction(list) | Re::Alternative(list) => list
                .iter()
                .fold(None, |acc, r| union(acc, r.capture_registers())),
            Re::Lookaround(l) => l.body.capture_registers(),
            Re::Capture { index, body } => {
                union(Some((2 * index, 2 * index + 1)), body.capture_registers())
            }
            Re::Quantifier { body, .. } | Re::Group { body, .. } => body.capture_registers(),
            // V8: the default (Interval::Empty) for every other tree, the class sets too.
            _ => None,
        }
    }

    /// V8 `IsCertainlyAnchoredAtStart` (`budget` is V8's recursion budget).
    pub(crate) fn is_certainly_anchored_at_start(&self, budget: i32) -> bool {
        match self {
            Re::Assertion(t) => *t == AssertionType::StartOfInput,
            Re::Alternative(nodes) => {
                if budget < 0 {
                    return false;
                }
                for node in nodes {
                    if node.is_certainly_anchored_at_start(budget - 1) {
                        return true;
                    }
                    if node.max_match() > 0 {
                        return false;
                    }
                }
                false
            }
            Re::Disjunction(alts) => {
                budget >= 0
                    && alts
                        .iter()
                        .all(|a| a.is_certainly_anchored_at_start(budget - 1))
            }
            Re::Lookaround(l) => {
                budget >= 0
                    && l.positive
                    && !l.lookbehind
                    && l.body.is_certainly_anchored_at_start(budget - 1)
            }
            Re::Capture { body, .. } => {
                budget >= 0 && body.is_certainly_anchored_at_start(budget - 1)
            }
            // V8: no override for Group, Quantifier and the rest (false).
            _ => false,
        }
    }

    /// True for V8's `IsAtom` or a `Text` that starts with an atom (V8 `StartsWithAtom`).
    pub(crate) fn starts_with_atom(&self) -> bool {
        match self {
            Re::Atom(_) => true,
            Re::Text(elements) => matches!(elements.first(), Some(TextRe::Atom(_))),
            _ => false,
        }
    }

    /// V8 `FirstAtom` (only after [`Re::starts_with_atom`]).
    pub(crate) fn first_atom(&self) -> &[u16] {
        match self {
            Re::Atom(units) => units,
            Re::Text(elements) => match elements.first() {
                Some(TextRe::Atom(units)) => units,
                _ => unreachable!("starts_with_atom"),
            },
            _ => unreachable!("starts_with_atom"),
        }
    }
}

/// Makes the compile-time tree of `tree` with `flags` in force.
pub(crate) fn lower(tree: &Tree, flags: Flags) -> Re {
    match tree {
        Tree::Disjunction(alts) => Re::Disjunction(alts.iter().map(|t| lower(t, flags)).collect()),
        Tree::Alternative(nodes) => {
            Re::Alternative(nodes.iter().map(|t| lower(t, flags)).collect())
        }
        Tree::Assertion(t) => Re::Assertion(*t),
        Tree::ClassRanges(cr) => Re::Class(lower_class(cr, flags)),
        Tree::ClassSetOperand(_) | Tree::ClassSetExpression(_) => lower_class_set(tree, flags),
        Tree::Atom(units) => Re::Atom(units.clone()),
        Tree::Text(elements) => Re::Text(
            elements
                .iter()
                .map(|e| match e {
                    Tree::Atom(units) => TextRe::Atom(units.clone()),
                    Tree::ClassRanges(cr) => TextRe::Class(lower_class(cr, flags)),
                    other => unreachable!("a text element: {other:?}"),
                })
                .collect(),
        ),
        Tree::Quantifier(q) => Re::Quantifier {
            min: q.min,
            max: q.max,
            greedy: q.kind == QuantifierType::Greedy,
            body: Box::new(lower(&q.body, flags)),
        },
        Tree::Capture(c) => Re::Capture {
            index: c.index,
            body: Box::new(lower(&c.body, flags)),
        },
        Tree::Group(g) => Re::Group {
            flags: g.flags,
            body: Box::new(lower(&g.body, g.flags)),
        },
        Tree::Lookaround(l) => Re::Lookaround(LookaroundRe {
            positive: l.positive,
            lookbehind: l.kind == LookaroundType::Lookbehind,
            capture_count: l.capture_count,
            capture_from: l.capture_from,
            body: Box::new(lower(&l.body, flags)),
        }),
        Tree::BackReference(b) => Re::BackReference(b.captures.clone()),
        Tree::Empty => Re::Empty,
    }
}

/// A class of the parser (no `v`): its ranges as V8's parser makes them (the case closure of
/// `\w` and `\p` with `u`+`i` included; the rest of the closure is the compiler's), and the
/// flags of a bracketed class (see the module doc).
fn lower_class(cr: &crate::ast::ClassRanges, flags: Flags) -> ClassRe {
    let mut ranges = Vec::new();
    for &item in &cr.items {
        sets::add_class_item(item, flags, &mut ranges);
    }
    ClassRe::new(
        ranges,
        cr.negated,
        !flags.ignore_case(),
        false,
        !flags.either_unicode(),
    )
}

/// V8 `ClassSetOperand` and `ClassSetExpression` min and max (the constructors).
fn class_set_min_max(tree: &Tree, flags: Flags) -> (u32, u32) {
    match tree {
        Tree::ClassSetOperand(_) => {
            let value = sets::class_set(tree, flags).expect("a class operand");
            let (mut min, mut max) = if value.ranges.is_empty() {
                (0, 0)
            } else {
                (1, 2)
            };
            for string in value.strings.values() {
                let string = lower(string, flags);
                min = min.min(string.min_match());
                max = max.max(string.max_match());
            }
            (min, max)
        }
        Tree::ClassSetExpression(ClassSetExpression {
            negated, operands, ..
        }) => {
            // V8: min_match is 0; a negated class has no strings and may match one
            // supplementary code point.
            let max = if *negated {
                2
            } else {
                operands
                    .iter()
                    .map(|o| class_set_min_max(o, flags).1)
                    .max()
                    .unwrap_or(0)
            };
            (0, max)
        }
        other => unreachable!("a v-mode class: {other:?}"),
    }
}

/// A `v`-mode class: the tree of V8 `ClassSetOperand::ToNodeImpl` over the value of
/// `ComputeExpression` (`sets::class_set`): the strings longest first, then the code points
/// as one class (folded already: `NO_CASE_FOLDING_NEEDED`), then the empty string.
fn lower_class_set(tree: &Tree, flags: Flags) -> Re {
    let (min, max) = class_set_min_max(tree, flags);
    let value = sets::class_set(tree, flags).expect("a v-mode class");
    let size = value.strings.len() + usize::from(!value.ranges.is_empty());
    if size == 0 {
        return Re::ClassSet {
            min,
            max,
            body: None,
        };
    }
    let mut alternatives = Vec::with_capacity(size);
    let mut empty = None;
    for string in value.strings.values() {
        if matches!(string, Tree::Empty) {
            empty = Some(Re::Empty);
        } else {
            alternatives.push(lower(string, flags));
        }
    }
    if !value.ranges.is_empty() {
        alternatives.push(Re::Class(ClassRe::new(
            value.ranges,
            false,
            true,
            false,
            false,
        )));
    }
    alternatives.extend(empty);
    let body = if alternatives.len() == 1 {
        alternatives.pop().expect("one alternative")
    } else {
        Re::Disjunction(alternatives)
    };
    Re::ClassSet {
        min,
        max,
        body: Some(Box::new(body)),
    }
}
