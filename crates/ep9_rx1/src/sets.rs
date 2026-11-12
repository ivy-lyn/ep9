// Copyright 2019 the V8 project authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// Ported for ep9 from V8 src/regexp/regexp-compiler-tonode.cc (the set part of
// ClassRanges::ToNodeImpl, ClassSetOperand::Union/Intersect/Subtract,
// ClassSetExpression::ComputeExpression), src/regexp/regexp-compiler.cc (the set part of
// TextNode::MakeCaseIndependent) and src/regexp/regexp-parser.cc (the ICU part of
// AddPropertyClassRange: LookupPropertyValueName, LookupSpecialPropertyValueName,
// AddUnicodeSetRanges, ExtractStringsFromUnicodeSet; AddMaybeSimpleCaseFoldedRange and the
// u_foldCase of ParseClassStringDisjunction) at 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0.
// The V8 licence text is in THIRD-PARTY-LICENSE (entry "V8 (BSD-3-Clause)").

//! The code point sets of the tree's class items (RX1 port step 2, docs/plan/04-execution.md
//! section 3).
//!
//! ```text
//! Tree::ClassRanges (no v)                  class_ranges(cr, flags, one_byte) -> ClassSet
//!   items -> ranges: Range as is; \d \s \w (AddClassEscape; \w \W closed first with u+i);
//!           \p{..} (property; never closed without v)
//!   no ranges -> everything, negated flips (V8 ClassRanges constructor)
//!   u or v, with i: + Simple_Case_Folding closure (ToNodeImpl)
//!   i without u:     + Canonicalize equivalents (TextNode::MakeCaseIndependent)
//!   negated: a flag of the set (V8 negates when it makes the node)
//! Tree::ClassSetOperand / ClassSetExpression (v)
//!                                           class_set(tree, flags) -> StringSet
//!   each leaf folds where V8's parser folds (v+i): characters and ranges
//!   (AddMaybeSimpleCaseFoldedRange), \p before \P's complement (AddUnicodeSetRanges),
//!   the strings of \q{} and of properties of strings (u_foldCase on each code point)
//!   then ComputeExpression: union, intersection, subtraction, complement
//! ```
//!
//! In V8 the parser makes the v-mode ranges (it holds ICU); in RX1 the parser keeps names
//! ([`ClassItem`]) and these functions make the ranges, at the same points of the
//! algorithm. A [`ClassSet`] matches the code point `c` (a code unit without `u`/`v`) when
//! `contains(&set.ranges, c) != set.negated`. The strings of a [`StringSet`] keep the trees
//! of the parser; a property of strings gives each string as an [`Tree::Atom`] of its UTF-16
//! code units (V8 builds them with its TextBuilder, which gives the same text: no emoji
//! string has a character with case equivalents).

use crate::ast::{
    ClassItem, ClassRanges, ClassSetExpression, ClassSetOperand, ClassSetOperation, ClassString,
    ClassStrings, Property, StandardSet, Tree,
};
use crate::charset::{self, CharacterRange};
use crate::flags::Flags;
use crate::special_case::{self, Mode};
use crate::unicode_sets::{
    BINARY, GENERAL_CATEGORY, Ranges, SCRIPT, SCRIPT_EXTENSIONS, STRING_PROPERTIES,
};

/// The code points of a [`Tree::ClassRanges`], after the case closure of its flags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassSet {
    /// Canonical ranges ([`charset::is_canonical`]).
    pub ranges: Vec<CharacterRange>,
    /// The class matches the code points not in `ranges`.
    pub negated: bool,
}

impl ClassSet {
    /// True if the class matches `c` (a code point with `u` or `v`, else a code unit).
    #[must_use]
    pub fn matches(&self, c: u32) -> bool {
        charset::contains(&self.ranges, c) != self.negated
    }
}

/// The value of a `v`-mode class (V8 `ClassSetOperand` after `ComputeExpression`): code
/// points and strings, after the case folding of `v`+`i`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct StringSet {
    /// Canonical ranges.
    pub ranges: Vec<CharacterRange>,
    /// The strings of any length but 1, folded with `i` (V8's key form), longest first.
    pub strings: ClassStrings,
}

fn to_ranges(table: Ranges) -> Vec<CharacterRange> {
    table
        .iter()
        .map(|&(f, t)| CharacterRange::range(f, t))
        .collect()
}

fn lookup(table: &'static [(&'static str, Ranges)], name: &str) -> Ranges {
    match table.binary_search_by(|(n, _)| (*n).cmp(name)) {
        Ok(i) => table[i].1,
        // The parser takes only names that the tables have (unicode.rs and unicode_sets.rs
        // come from one PropertyValueAliases.txt; a unit test checks every name).
        Err(_) => unreachable!("no set for the property value {name}"),
    }
}

/// The code points and the strings of a Unicode property (ICU `applyIntPropertyValue`,
/// `applyPropertyAlias`), before any case closure or complement. The strings are code point
/// lists of length 2 or more.
#[must_use]
pub fn property_members(property: Property) -> (Vec<CharacterRange>, Vec<&'static [u32]>) {
    match property {
        Property::GeneralCategory(v) => (to_ranges(lookup(GENERAL_CATEGORY, v)), Vec::new()),
        Property::Script(v) => (to_ranges(lookup(SCRIPT, v)), Vec::new()),
        Property::ScriptExtensions(v) => (to_ranges(lookup(SCRIPT_EXTENSIONS, v)), Vec::new()),
        Property::Binary(name) => (to_ranges(lookup(BINARY, name)), Vec::new()),
        Property::Any => (vec![CharacterRange::everything()], Vec::new()),
        Property::Ascii => (vec![CharacterRange::range(0, 0x7f)], Vec::new()),
        Property::Assigned => (
            charset::negate(&to_ranges(lookup(GENERAL_CATEGORY, "Cn"))),
            Vec::new(),
        ),
        Property::OfStrings(name) => {
            let mut ranges = Vec::new();
            let mut strings: Vec<&'static [u32]> = Vec::new();
            for p in STRING_PROPERTIES {
                // RGI_Emoji is the union of the other six (ECMA-262 table 69; UTS #51 ED-27).
                if name == "RGI_Emoji" || p.name == name {
                    ranges.extend(to_ranges(p.singles));
                    strings.extend_from_slice(p.strings);
                }
            }
            charset::canonicalize(&mut ranges);
            strings.sort_unstable();
            strings.dedup();
            (ranges, strings)
        }
    }
}

/// The ranges and strings of `\p{..}` (`negate: false`) or `\P{..}` (V8
/// `AddPropertyClassRange` with ICU): with `v`+`i` the set is closed under Simple_Case_Folding
/// before the complement (V8 `AddUnicodeSetRanges`), and each string is folded (V8
/// `ExtractStringsFromUnicodeSet`). `Any`, `ASCII` and `Assigned` follow V8's
/// `LookupSpecialPropertyValueName`: `\P{Any}` is empty, `ASCII` takes the closure too, and
/// `Assigned` is the complement of `gc=Cn` (after its closure).
fn add_property(
    property: Property,
    negate: bool,
    flags: Flags,
    ranges: &mut Vec<CharacterRange>,
    strings: &mut ClassStrings,
) {
    let (property, negate) = match property {
        Property::Any => {
            if !negate {
                ranges.push(CharacterRange::everything());
            }
            return;
        }
        // V8: LookupPropertyValueName(gc, "Unassigned", !negate).
        Property::Assigned => (Property::GeneralCategory("Cn"), !negate),
        p => (p, negate),
    };
    let (mut set, set_strings) = property_members(property);
    // Strings come only with `v` (the parser refuses \P of a property of strings).
    debug_assert!(set_strings.is_empty() || (flags.unicode_sets() && !negate));
    for s in set_strings {
        let key: Vec<u32> = if flags.ignore_case() {
            s.iter().map(|&c| special_case::simple_fold(c)).collect()
        } else {
            s.to_vec()
        };
        strings
            .entry(ClassString(key))
            .or_insert_with(|| string_tree(s));
    }
    if flags.unicode_sets() && flags.ignore_case() {
        special_case::close_over(&mut set, Mode::Unicode);
    }
    if negate {
        set = charset::negate(&set);
    }
    ranges.extend(set);
}

/// The tree of one string of a property of strings: its UTF-16 code units.
fn string_tree(cps: &[u32]) -> Tree {
    let mut units = Vec::with_capacity(cps.len() * 2);
    for &c in cps {
        if let Some(ch) = char::from_u32(c) {
            let mut buf = [0u16; 2];
            units.extend_from_slice(ch.encode_utf16(&mut buf));
        }
    }
    Tree::Atom(units)
}

/// The ranges of a non-`v` class item, as V8's parser adds them (`ParseClassRanges`,
/// `TryParseCharacterClassEscape`).
pub(crate) fn add_class_item(item: ClassItem, flags: Flags, ranges: &mut Vec<CharacterRange>) {
    match item {
        ClassItem::Range(from, to) => ranges.push(CharacterRange::range(from, to)),
        ClassItem::Standard(set) => {
            // V8 passes add_unicode_case_equivalents() for \d \s \w, and false for `.`.
            let closure = !matches!(
                set,
                StandardSet::NotLineTerminator | StandardSet::Everything
            ) && flags.either_unicode()
                && flags.ignore_case();
            charset::add_class_escape(set, ranges, closure);
        }
        ClassItem::Property { property, negated } => {
            let mut no_strings = ClassStrings::new();
            add_property(property, negated, flags, ranges, &mut no_strings);
            debug_assert!(no_strings.is_empty());
        }
    }
}

/// The code points of a non-`v` class ([`Tree::ClassRanges`]) with the case closure of
/// `flags` (V8 `ClassRanges::ToNodeImpl` with `u`/`v`+`i`, or `TextNode::MakeCaseIndependent`
/// with `i` alone; `one_byte` is V8's `is_one_byte` of `AddCaseEquivalents`, the
/// Latin-1-subject program). The negation stays a flag, as in V8.
#[must_use]
pub fn class_ranges(cr: &ClassRanges, flags: Flags, one_byte: bool) -> ClassSet {
    let mut ranges = Vec::new();
    for &item in &cr.items {
        add_class_item(item, flags, &mut ranges);
    }
    let mut negated = cr.negated;
    // V8 ClassRanges constructor: the empty set is the negated Everything.
    if ranges.is_empty() {
        ranges.push(CharacterRange::everything());
        negated = !negated;
    }
    charset::canonicalize(&mut ranges);
    if flags.ignore_case() {
        if flags.either_unicode() {
            charset::add_unicode_case_equivalents(&mut ranges);
        } else {
            charset::add_case_equivalents(&mut ranges, one_byte);
            charset::canonicalize(&mut ranges);
        }
    }
    ClassSet { ranges, negated }
}

/// V8 `AddMaybeSimpleCaseFoldedRange`: with `i` (in `v` mode), the closure of the range.
fn add_maybe_folded(range: CharacterRange, flags: Flags, ranges: &mut Vec<CharacterRange>) {
    if flags.ignore_case() {
        let mut one = vec![range];
        charset::add_unicode_case_equivalents(&mut one);
        ranges.extend(one);
    } else {
        ranges.push(range);
    }
}

/// The value of a `v`-mode operand, as V8's parser builds its `ClassSetOperand`.
fn operand_value(op: &ClassSetOperand, flags: Flags) -> StringSet {
    let mut ranges = Vec::new();
    let mut strings = ClassStrings::new();
    for &item in &op.items {
        match item {
            ClassItem::Range(from, to) => {
                add_maybe_folded(CharacterRange::range(from, to), flags, &mut ranges);
            }
            ClassItem::Standard(set) => {
                charset::add_class_escape(set, &mut ranges, flags.ignore_case());
            }
            ClassItem::Property { property, negated } => {
                add_property(property, negated, flags, &mut ranges, &mut strings);
            }
        }
    }
    for (key, tree) in &op.strings {
        // V8 folds each code point of a \q{} string with u_foldCase under `i`.
        let key = if flags.ignore_case() {
            ClassString(
                key.0
                    .iter()
                    .map(|&c| special_case::simple_fold(c))
                    .collect(),
            )
        } else {
            key.clone()
        };
        strings.entry(key).or_insert_with(|| tree.clone());
    }
    charset::canonicalize(&mut ranges);
    StringSet { ranges, strings }
}

/// The value of a `v`-mode class: a [`Tree::ClassSetOperand`] or a
/// [`Tree::ClassSetExpression`] (V8 `ClassSetExpression::ComputeExpression`). Any other tree
/// is not a class and gives `None`.
#[must_use]
pub fn class_set(tree: &Tree, flags: Flags) -> Option<StringSet> {
    match tree {
        Tree::ClassSetOperand(op) => Some(operand_value(op, flags)),
        Tree::ClassSetExpression(e) => Some(expression_value(e, flags)),
        _ => None,
    }
}

#[cfg(test)]
thread_local! {
    /// The elements the subtraction visits on this thread (the test of its growth).
    pub(crate) static SUBTRACTION_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn count_work(n: usize) {
    SUBTRACTION_WORK.with(|w| w.set(w.get() + n));
}

#[cfg(not(test))]
const fn count_work(_: usize) {}

/// V8 `ComputeExpression` (the nesting is at most `MAX_NESTING` levels).
fn expression_value(e: &ClassSetExpression, flags: Flags) -> StringSet {
    let mut operands = e
        .operands
        .iter()
        .map(|t| class_set(t, flags).unwrap_or_else(|| unreachable!("a class operand: {t:?}")));
    let mut result = operands.next().unwrap_or_default();
    match e.operation {
        ClassSetOperation::Union => {
            for op in operands {
                result.ranges.extend(op.ranges);
                for (k, v) in op.strings {
                    result.strings.entry(k).or_insert(v);
                }
            }
            charset::canonicalize(&mut result.ranges);
        }
        ClassSetOperation::Intersection => {
            for op in operands {
                result.ranges = charset::intersect(&result.ranges, &op.ranges);
                result.strings.retain(|k, _| op.strings.contains_key(k));
            }
        }
        ClassSetOperation::Subtraction => {
            // A - B - C == A - (B | C): one subtraction of the union of the right operands,
            // and each string of an operand removed by its key. The work grows with the
            // operand sizes, not with their product (V8 rescans the set per operand).
            let mut remove = Vec::new();
            for op in operands {
                count_work(op.ranges.len() + op.strings.len());
                remove.extend(op.ranges);
                for k in op.strings.keys() {
                    result.strings.remove(k);
                }
            }
            charset::canonicalize(&mut remove);
            count_work(result.ranges.len() + remove.len());
            result.ranges = charset::subtract(&result.ranges, &remove);
        }
    }
    if e.negated {
        debug_assert!(result.strings.is_empty());
        result.ranges = charset::negate(&result.ranges);
    }
    result
}
