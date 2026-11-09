// Copyright 2016 the V8 project authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// Ported for ep9 from V8 src/regexp/regexp-ast.h at 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0.
// The V8 licence text is in THIRD-PARTY-LICENSE (entry "V8 (BSD-3-Clause)").

//! The tree that the parser builds (the node kinds of V8 `regexp-ast.h`).
//!
//! ```text
//! V8 (zone objects, pointers)            RX1 (owned values)
//! Tree subclasses                        Tree enum, one variant per subclass
//! CharacterRange lists (ICU sets)        ClassItem lists: ranges, \d \s \w ., \p{..} by name
//! Capture objects shared by references   Tree::Capture in place; BackReference holds indices
//! CharacterClassStrings (a ZoneMap)      ClassStrings (a BTreeMap with V8's order)
//! ```
//!
//! The parser keeps class escapes and property escapes by name ([`ClassItem`]). V8 turns them
//! into code point ranges in the parser (`CharacterRange::AddClassEscape`, ICU sets, the case
//! closure under `i`). In RX1 that step is the compiler's (RX1-1), with the Unicode sets.
//! `min_match` and `max_match` are not stored; [`Tree::max_match_is_zero`] is the one question
//! the parser asks (V8 `Builder::AddQuantifierToAtom`).

use std::collections::BTreeMap;

use crate::flags::Flags;

/// V8 `Tree::kInfinity`: an unbounded quantifier maximum.
pub const INFINITY: u32 = i32::MAX as u32;

/// A node of the pattern tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tree {
    /// `a|b|...` (two or more alternatives).
    Disjunction(Vec<Tree>),
    /// A sequence of two or more terms.
    Alternative(Vec<Tree>),
    /// `^`, `$`, `\b`, `\B`.
    Assertion(AssertionType),
    /// A character class without `v`, or a class escape or `.` outside a class.
    ClassRanges(ClassRanges),
    /// A `v`-mode class operand (V8 `ClassSetOperand`): code points and strings.
    ClassSetOperand(ClassSetOperand),
    /// A `v`-mode class (V8 `ClassSetExpression`).
    ClassSetExpression(ClassSetExpression),
    /// Literal code units.
    Atom(Vec<u16>),
    /// Two or more text elements ([`Tree::Atom`] or [`Tree::ClassRanges`]).
    Text(Vec<Tree>),
    /// A quantified atom.
    Quantifier(Quantifier),
    /// A capturing group.
    Capture(Capture),
    /// A non-capturing group `(?:...)`, with its modifiers applied (`(?i:...)`).
    Group(Group),
    /// A look-ahead or look-behind.
    Lookaround(Lookaround),
    /// `\1` or `\k<name>`.
    BackReference(BackReference),
    /// The empty pattern.
    Empty,
}

/// The kinds of [`Tree::Assertion`] (V8 `Assertion::Type`; `END_OF_BUFFER` comes only
/// with V8's `--js-regexp-buffer-boundaries` flag, which RX1 does not port).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssertionType {
    /// `^` with `m`.
    StartOfLine,
    /// `^`.
    StartOfInput,
    /// `$` with `m`.
    EndOfLine,
    /// `$`.
    EndOfInput,
    /// `\b`.
    Boundary,
    /// `\B`.
    NonBoundary,
}

/// The named standard sets (V8 `StandardCharacterSet`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StandardSet {
    /// `\s`.
    Whitespace,
    /// `\S`.
    NotWhitespace,
    /// `\w`.
    Word,
    /// `\W`.
    NotWord,
    /// `\d`.
    Digit,
    /// `\D`.
    NotDigit,
    /// `.` without `s`.
    NotLineTerminator,
    /// `.` with `s`.
    Everything,
}

impl StandardSet {
    /// The set of the escape letter `c` (`d D s S w W`).
    pub(crate) fn of_escape(c: u32) -> Option<StandardSet> {
        Some(match char::from_u32(c)? {
            'd' => StandardSet::Digit,
            'D' => StandardSet::NotDigit,
            's' => StandardSet::Whitespace,
            'S' => StandardSet::NotWhitespace,
            'w' => StandardSet::Word,
            'W' => StandardSet::NotWord,
            _ => return None,
        })
    }

    /// True if the set holds a code point above U+FFFF (V8 `NeedsDesugaringForUnicode`).
    fn has_non_bmp(self) -> bool {
        matches!(
            self,
            StandardSet::NotWhitespace
                | StandardSet::NotWord
                | StandardSet::NotDigit
                | StandardSet::NotLineTerminator
                | StandardSet::Everything
        )
    }
}

/// A Unicode property of `\p{...}` and `\P{...}`, by its short name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Property {
    /// A General_Category value (`\p{Lu}`, `\p{gc=Letter}`).
    GeneralCategory(&'static str),
    /// A Script value (`\p{sc=Grek}`).
    Script(&'static str),
    /// A Script_Extensions value (`\p{scx=Grek}`).
    ScriptExtensions(&'static str),
    /// A binary property, by its canonical name (`\p{Alpha}` is `Alphabetic`).
    Binary(&'static str),
    /// A binary property of strings (`v` only; `\p{RGI_Emoji}`).
    OfStrings(&'static str),
    /// `Any`.
    Any,
    /// `ASCII`.
    Ascii,
    /// `Assigned`.
    Assigned,
}

/// One element of a class: code points or a named set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClassItem {
    /// The code points `from..=to`.
    Range(u32, u32),
    /// `\d`, `\s`, `\w`, `.` and their complements.
    Standard(StandardSet),
    /// `\p{..}` (`negated: false`) or `\P{..}` (`negated: true`).
    Property {
        /// The property.
        property: Property,
        /// `\P`.
        negated: bool,
    },
}

impl ClassItem {
    /// True if the item may hold a code point above U+FFFF or a surrogate (V8
    /// `NeedsDesugaringForUnicode` on the ranges; a property counts as may).
    fn may_need_desugaring(self) -> bool {
        match self {
            ClassItem::Range(from, to) => to >= 0x10000 || (from <= 0xdfff && to >= 0xd800),
            ClassItem::Standard(s) => s.has_non_bmp(),
            ClassItem::Property { .. } => true,
        }
    }
}

/// A class of single code points (V8 `ClassRanges`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassRanges {
    /// The items, in pattern order (not canonicalized).
    pub items: Vec<ClassItem>,
    /// `[^...]`.
    pub negated: bool,
}

impl ClassRanges {
    /// V8 `TextBuilder::NeedsDesugaringForUnicode`: in Unicode mode a class that may match a
    /// surrogate pair or a lone surrogate is its own term, not part of a [`Tree::Text`]. This
    /// form only decides the tree shape; a negated class always counts.
    pub(crate) fn needs_desugaring_for_unicode(&self, flags: Flags) -> bool {
        if !flags.either_unicode() {
            return false;
        }
        if flags.ignore_case() || self.negated {
            return true;
        }
        self.items.iter().any(|i| i.may_need_desugaring())
    }
}

/// The strings of a `v`-mode class: code points -> the string as a tree (V8
/// `CharacterClassStrings`; the key order is V8's, longest first).
pub type ClassStrings = BTreeMap<ClassString, Tree>;

/// A key of [`ClassStrings`]: longer strings sort first, then by code point (V8
/// `CharacterClassStringLess`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassString(pub Vec<u32>);

impl Ord for ClassString {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .0
            .len()
            .cmp(&self.0.len())
            .then_with(|| self.0.cmp(&other.0))
    }
}

impl PartialOrd for ClassString {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// A `v`-mode class operand (V8 `ClassSetOperand`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ClassSetOperand {
    /// The code point items. A property of strings is an item too ([`Property::OfStrings`]):
    /// its strings come with the Unicode set (RX1-1).
    pub items: Vec<ClassItem>,
    /// The strings of `\q{..}` (each alternative whose length is not 1).
    pub strings: ClassStrings,
}

/// The set operation of a [`ClassSetExpression`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClassSetOperation {
    /// `[ab]`.
    Union,
    /// `[a&&b]`.
    Intersection,
    /// `[a--b]`.
    Subtraction,
}

/// A `v`-mode class (V8 `ClassSetExpression`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassSetExpression {
    /// The operation.
    pub operation: ClassSetOperation,
    /// `[^...]`.
    pub negated: bool,
    /// MayContainStrings (ECMA-262 22.2.1.6).
    pub may_contain_strings: bool,
    /// The operands: [`Tree::ClassSetOperand`] or [`Tree::ClassSetExpression`].
    pub operands: Vec<Tree>,
}

impl ClassSetExpression {
    /// V8 `ClassSetExpression::Empty`: `[]` (nothing) or `[^]` (everything).
    pub(crate) fn empty(negated: bool) -> Tree {
        Tree::ClassSetExpression(ClassSetExpression {
            operation: ClassSetOperation::Union,
            negated,
            may_contain_strings: false,
            operands: vec![Tree::ClassSetOperand(ClassSetOperand::default())],
        })
    }
}

/// The kinds of [`Quantifier`] (V8 `Quantifier::QuantifierType`; `POSSESSIVE` comes only
/// with a V8 debug flag and is not ported).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QuantifierType {
    /// `a*`.
    Greedy,
    /// `a*?`.
    NonGreedy,
}

/// A quantified atom (V8 `Quantifier`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quantifier {
    /// The minimum count.
    pub min: u32,
    /// The maximum count ([`INFINITY`] for none).
    pub max: u32,
    /// Greedy or not.
    pub kind: QuantifierType,
    /// The quantifier's number in pattern order (V8 `index`).
    pub index: u32,
    /// The atom.
    pub body: Box<Tree>,
}

/// A capturing group (V8 `Capture`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    /// The group number (1 for the first group).
    pub index: u32,
    /// The group name (code units), if any.
    pub name: Option<Vec<u16>>,
    /// The body.
    pub body: Box<Tree>,
}

/// A non-capturing group (V8 `Group`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// The flags inside the group (the modifiers applied).
    pub flags: Flags,
    /// The body.
    pub body: Box<Tree>,
}

/// The direction of a [`Lookaround`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LookaroundType {
    /// `(?=` and `(?!`.
    Lookahead,
    /// `(?<=` and `(?<!`.
    Lookbehind,
}

/// A look-around assertion (V8 `Lookaround`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lookaround {
    /// `(?=`, `(?<=`.
    pub positive: bool,
    /// The direction.
    pub kind: LookaroundType,
    /// The number of captures inside.
    pub capture_count: u32,
    /// The first capture index inside (V8 `capture_from`).
    pub capture_from: u32,
    /// The look-around's number in pattern order (V8 `index`).
    pub index: u32,
    /// The body.
    pub body: Box<Tree>,
}

/// A back reference (V8 `BackReference`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackReference {
    /// The groups it refers to: one for `\n`; for `\k<name>` every group of that name.
    pub captures: Vec<u32>,
    /// The name of `\k<name>`.
    pub name: Option<Vec<u16>>,
}

impl Tree {
    /// True for the text elements (V8 `IsTextElement`).
    pub fn is_text_element(&self) -> bool {
        matches!(
            self,
            Tree::Atom(_)
                | Tree::Text(_)
                | Tree::ClassRanges(_)
                | Tree::ClassSetOperand(_)
                | Tree::ClassSetExpression(_)
        )
    }

    /// True if the node only ever matches the empty string (V8 `max_match() == 0`). A class
    /// with a property item counts as non-empty (the ranges are known only to the compiler);
    /// that keeps a quantifier V8 may drop, which matches the same strings.
    pub fn max_match_is_zero(&self) -> bool {
        match self {
            Tree::Assertion(_) | Tree::Lookaround(_) | Tree::Empty => true,
            Tree::Atom(units) => units.is_empty(),
            Tree::Text(elements) => elements.iter().all(Tree::max_match_is_zero),
            Tree::ClassRanges(_) => false,
            Tree::ClassSetOperand(op) => {
                op.items.is_empty() && op.strings.values().all(Tree::max_match_is_zero)
            }
            Tree::ClassSetExpression(e) => {
                !e.negated && e.operands.iter().all(Tree::max_match_is_zero)
            }
            Tree::Disjunction(alts) | Tree::Alternative(alts) => {
                alts.iter().all(Tree::max_match_is_zero)
            }
            Tree::Quantifier(q) => q.max == 0 || q.body.max_match_is_zero(),
            Tree::Capture(c) => c.body.max_match_is_zero(),
            Tree::Group(g) => g.body.max_match_is_zero(),
            Tree::BackReference(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_strings_sort_longest_first() {
        let mut m = ClassStrings::new();
        m.insert(ClassString(vec![1]), Tree::Empty);
        m.insert(ClassString(vec![2, 3]), Tree::Empty);
        m.insert(ClassString(vec![1, 4]), Tree::Empty);
        let keys: Vec<Vec<u32>> = m.keys().map(|k| k.0.clone()).collect();
        assert_eq!(keys, vec![vec![1, 4], vec![2, 3], vec![1]]);
    }

    #[test]
    fn max_match_is_zero_follows_v8() {
        assert!(Tree::Empty.max_match_is_zero());
        assert!(Tree::Assertion(AssertionType::Boundary).max_match_is_zero());
        assert!(!Tree::Atom(vec![0x61]).max_match_is_zero());
        assert!(ClassSetExpression::empty(false).max_match_is_zero());
        assert!(!ClassSetExpression::empty(true).max_match_is_zero());
        let q = Tree::Quantifier(Quantifier {
            min: 0,
            max: 0,
            kind: QuantifierType::Greedy,
            index: 0,
            body: Box::new(Tree::Atom(vec![0x61])),
        });
        assert!(q.max_match_is_zero());
    }
}
