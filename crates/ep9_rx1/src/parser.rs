// Copyright 2016 the V8 project authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// Ported for ep9 from V8 src/regexp/regexp-parser.cc at 40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0.
// The V8 licence text is in THIRD-PARTY-LICENSE (entry "V8 (BSD-3-Clause)").

//! The RegExp pattern parser (V8 `regexp-parser.cc`, the `kBuildAST` mode).
//!
//! The structure is V8's, function by function, so a V8 change maps to one place here:
//!
//! | V8 | Here |
//! |---|---|
//! | `TextBuilder<kBuildAST>` | [`TextBuilder`] |
//! | `Builder<kBuildAST>` | [`Builder`] |
//! | `ParserState` (a linked list) | [`ParserState`] (a `Vec` stack in [`Parser::states`]) |
//! | `ParserImpl<CharT, kBuildAST>` | [`Parser`], generic over the code unit ([`Unit`]: `u8` Latin-1 or `u16`) |
//! | `ReportError` + `CHECK_FAILED` | [`Parser::report_error`] returns [`Failed`]; `?` |
//! | `ClassSetResult` (`Failure()` with and without an error) | `PResult<Option<ClassSetResult>>`: `Ok(None)` is "not this form", `Err` an error |
//! | `Capture*` shared by `BackReference` | indices; [`Parser::patch_named_back_references`] fills them |
//!
//! Not ported (each is a named gap, listed in the crate doc): the `kVerifySyntax` mode (one
//! mode here), the `--js-regexp-buffer-boundaries` escapes `\A \z \Z` and the debug-only
//! possessive quantifier (both V8 flags are off by default), the code point ranges of class
//! and property escapes (the compiler's job in RX1-1), the case-fold desugaring of `u`+`i`
//! and `v`+`i` (`NeedsDesugaringForIgnoreCase`, `AddMaybeSimpleCaseFoldedRange`,
//! `u_foldCase` in `\q{}`: tree shape only, no effect on what parses), and V8's
//! stack-limit check in `Advance` (here [`MAX_NESTING`] caps groups and nested classes).

use crate::ast::{
    AssertionType, BackReference, Capture, ClassItem, ClassRanges, ClassSetExpression,
    ClassSetOperand, ClassSetOperation, ClassString, ClassStrings, Group, INFINITY, Lookaround,
    LookaroundType, Property, Quantifier, QuantifierType, StandardSet, Tree,
};
use crate::error::{Error, ParseError};
use crate::flags::Flags;
use crate::unicode;
use std::collections::BTreeMap;

/// V8 `RegExpMacroAssembler::kMaxCaptures`: `((1 << 16) - 1 - 1) / 2`.
pub const MAX_CAPTURES: u32 = ((1 << 16) - 1 - 1) / 2;

/// The deepest nesting of groups plus `v`-mode classes. Deeper patterns fail with
/// [`Error::StackOverflow`] (V8 has no fixed cap: its `Advance` checks the stack limit). The
/// nested-class parse and the tree's drop recurse once per level. At the cap a debug build
/// needs less than 1 MiB of stack (M7-P4: 1 MiB passes and 512 KiB fails in debug; 512 KiB
/// passes in release), so the cap fits a 1 MiB thread (the unit test runs on one).
pub const MAX_NESTING: usize = 256;

#[cfg(test)]
std::thread_local! {
    /// The interval steps of the duplicate-name scan on this thread (the cost test).
    pub(crate) static INTERVAL_STEPS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// End of input (V8 `kEndMarker`).
const END: u32 = 1 << 21;

/// A pattern code unit: `u8` (a Latin-1 string) or `u16` (UTF-16, lone surrogates allowed).
pub trait Unit: Copy {
    /// True for `u16` (V8 `sizeof(CharT) == 2`): only then can surrogates pair.
    const WIDE: bool;
    /// The code unit.
    fn unit(self) -> u16;
}

impl Unit for u8 {
    const WIDE: bool = false;
    fn unit(self) -> u16 {
        u16::from(self)
    }
}

impl Unit for u16 {
    const WIDE: bool = true;
    fn unit(self) -> u16 {
        self
    }
}

/// The parse of a pattern (V8 `CompileData`, the parser's fields).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    /// The tree.
    pub tree: Tree,
    /// The number of capturing groups.
    pub capture_count: u32,
    /// Each named group as (name, index), sorted by name, then by index (V8
    /// `GetNamedCaptures`).
    pub named_captures: Vec<(Vec<u16>, u32)>,
    /// The pattern is one atom with no capture and no escape (a literal search string).
    pub simple: bool,
    /// The pattern has `^`.
    pub contains_anchor: bool,
}

/// An error was reported; the details are in the parser.
#[derive(Debug)]
pub(crate) struct Failed;

type PResult<T> = Result<T, Failed>;

fn is_lead(c: u32) -> bool {
    (0xd800..=0xdbff).contains(&c)
}

fn is_trail(c: u32) -> bool {
    (0xdc00..=0xdfff).contains(&c)
}

fn combine(lead: u32, trail: u32) -> u32 {
    0x10000 + ((lead - 0xd800) << 10) + (trail - 0xdc00)
}

fn is_decimal_digit(c: u32) -> bool {
    (u32::from(b'0')..=u32::from(b'9')).contains(&c)
}

fn hex_value(c: u32) -> Option<u32> {
    char::from_u32(c)?.to_digit(16)
}

fn ch(c: u8) -> u32 {
    u32::from(c)
}

// ---- TextBuilder ---------------------------------------------------------------------------

/// Text of one alternative: literal units become [`Tree::Atom`]s; atoms and classes are joined
/// into a [`Tree::Text`] (V8 `TextBuilder`). The terms list belongs to the caller.
#[derive(Debug, Default)]
struct TextBuilder {
    flags: Flags,
    characters: Option<Vec<u16>>,
    text: Vec<Tree>,
}

impl TextBuilder {
    fn new(flags: Flags) -> TextBuilder {
        TextBuilder {
            flags,
            characters: None,
            text: Vec::new(),
        }
    }

    fn flush_characters(&mut self) {
        if let Some(chars) = self.characters.take() {
            self.text.push(Tree::Atom(chars));
        }
    }

    fn flush_text(&mut self, terms: &mut Vec<Tree>) {
        self.flush_characters();
        match self.text.len() {
            0 => {}
            1 => terms.push(self.text.pop().unwrap()),
            _ => {
                // V8 AppendToText: atoms and classes are the elements; a Text adds its own.
                let mut elements = Vec::with_capacity(self.text.len());
                for t in self.text.drain(..) {
                    match t {
                        Tree::Text(inner) => elements.extend(inner),
                        other => elements.push(other),
                    }
                }
                terms.push(Tree::Text(elements));
            }
        }
    }

    fn add_character(&mut self, c: u16) {
        // V8 NeedsDesugaringForIgnoreCase is false without `u`/`v` + `i`; with them it asks
        // ICU's case closure. Not ported (tree shape only; RX1-1 adds it with the case data).
        self.characters.get_or_insert_with(Vec::new).push(c);
    }

    fn add_unicode_character(&mut self, terms: &mut Vec<Tree>, c: u32) {
        if c > 0xffff {
            debug_assert!(self.flags.either_unicode());
            let lead = 0xd800 + ((c - 0x10000) >> 10);
            let trail = 0xdc00 + ((c - 0x10000) & 0x3ff);
            self.add_atom(Tree::Atom(vec![lead as u16, trail as u16]));
        } else if self.flags.either_unicode() && (is_lead(c) || is_trail(c)) {
            self.add_class_ranges_for_desugaring(terms, c);
        } else {
            self.add_character(c as u16);
        }
    }

    fn add_class_ranges(&mut self, terms: &mut Vec<Tree>, cr: ClassRanges) {
        if cr.needs_desugaring_for_unicode(self.flags) {
            // With `u` or `v` such a class is its own term, not part of a Text.
            self.add_term(terms, Tree::ClassRanges(cr));
        } else {
            self.add_atom(Tree::ClassRanges(cr));
        }
    }

    fn add_class_ranges_for_desugaring(&mut self, terms: &mut Vec<Tree>, c: u32) {
        self.add_term(
            terms,
            Tree::ClassRanges(ClassRanges {
                items: vec![ClassItem::Range(c, c)],
                negated: false,
            }),
        );
    }

    fn add_atom(&mut self, atom: Tree) {
        debug_assert!(atom.is_text_element());
        self.flush_characters();
        self.text.push(atom);
    }

    fn add_term(&mut self, terms: &mut Vec<Tree>, term: Tree) {
        debug_assert!(term.is_text_element());
        self.flush_text(terms);
        terms.push(term);
    }

    fn pop_last_atom(&mut self) -> Option<Tree> {
        if let Some(mut chars) = self.characters.take() {
            if chars.len() > 1 {
                let last = chars.pop().unwrap();
                self.text.push(Tree::Atom(chars));
                return Some(Tree::Atom(vec![last]));
            }
            return Some(Tree::Atom(chars));
        }
        self.text.pop()
    }

    /// V8 `ToRegExp`: the terms as one tree (the terms list is left empty).
    fn take_regexp(&mut self, terms: &mut Vec<Tree>) -> Tree {
        self.flush_text(terms);
        match terms.len() {
            0 => Tree::Empty,
            1 => terms.pop().unwrap(),
            _ => Tree::Alternative(std::mem::take(terms)),
        }
    }
}

// ---- Builder -------------------------------------------------------------------------------

/// The terms and alternatives of one disjunction (V8 `Builder`).
#[derive(Debug)]
struct Builder {
    pending_empty: bool,
    flags: Flags,
    terms: Vec<Tree>,
    alternatives: Vec<Tree>,
    text: TextBuilder,
}

impl Builder {
    fn new(flags: Flags) -> Builder {
        Builder {
            pending_empty: false,
            flags,
            terms: Vec::new(),
            alternatives: Vec::new(),
            text: TextBuilder::new(flags),
        }
    }

    fn flush_text(&mut self) {
        self.text.flush_text(&mut self.terms);
    }

    fn add_character(&mut self, c: u16) {
        self.pending_empty = false;
        self.text.add_character(c);
    }

    fn add_unicode_character(&mut self, c: u32) {
        self.pending_empty = false;
        self.text.add_unicode_character(&mut self.terms, c);
    }

    /// "Adds" an empty expression: nothing but a following quantifier is eaten.
    fn add_empty(&mut self) {
        self.pending_empty = true;
    }

    fn add_class_ranges(&mut self, cr: ClassRanges) {
        self.pending_empty = false;
        self.text.add_class_ranges(&mut self.terms, cr);
    }

    fn add_atom(&mut self, term: Tree) {
        if matches!(term, Tree::Empty) {
            self.add_empty();
            return;
        }
        self.pending_empty = false;
        if term.is_text_element() {
            self.text.add_atom(term);
        } else {
            self.flush_text();
            self.terms.push(term);
        }
    }

    fn add_term(&mut self, term: Tree) {
        self.pending_empty = false;
        debug_assert!(!matches!(term, Tree::Empty));
        if term.is_text_element() {
            self.text.add_term(&mut self.terms, term);
        } else {
            self.flush_text();
            self.terms.push(term);
        }
    }

    fn add_assertion(&mut self, assertion: Tree) {
        self.pending_empty = false;
        self.flush_text();
        self.terms.push(assertion);
    }

    fn new_alternative(&mut self) {
        self.flush_terms();
    }

    fn flush_terms(&mut self) {
        self.flush_text();
        let alternative = match self.terms.len() {
            0 => Tree::Empty,
            1 => self.terms.pop().unwrap(),
            _ => Tree::Alternative(std::mem::take(&mut self.terms)),
        };
        self.alternatives.push(alternative);
        self.terms.clear();
    }

    /// V8 `ToRegExp`: the alternatives as one tree (the builder is left empty).
    fn take_regexp(&mut self) -> Tree {
        self.flush_terms();
        match self.alternatives.len() {
            0 => Tree::Empty,
            1 => self.alternatives.pop().unwrap(),
            _ => Tree::Disjunction(std::mem::take(&mut self.alternatives)),
        }
    }

    /// V8 `AddQuantifierToAtom`: false when the atom may not take a quantifier.
    fn add_quantifier_to_atom(
        &mut self,
        min: u32,
        max: u32,
        index: u32,
        kind: QuantifierType,
    ) -> bool {
        if self.pending_empty {
            self.pending_empty = false;
            return true;
        }
        let atom = if let Some(atom) = self.text.pop_last_atom() {
            self.flush_text();
            atom
        } else if let Some(atom) = self.terms.pop() {
            if let Tree::Lookaround(l) = &atom {
                // With /u or /v, lookarounds are not quantifiable.
                if self.flags.either_unicode() {
                    return false;
                }
                // Lookbehinds are not quantifiable.
                if l.kind == LookaroundType::Lookbehind {
                    return false;
                }
            }
            if atom.max_match_is_zero() {
                // Guaranteed to only match an empty string.
                if min == 0 {
                    return true;
                }
                self.terms.push(atom);
                return true;
            }
            atom
        } else {
            // V8: UNREACHABLE (only called right after an atom or a character).
            unreachable!("a quantifier with no atom before it");
        };
        self.terms.push(Tree::Quantifier(Quantifier {
            min,
            max,
            kind,
            index,
            body: Box::new(atom),
        }));
        true
    }
}

// ---- ParserState ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubexpressionType {
    Initial,
    Capture,
    PositiveLookaround,
    NegativeLookaround,
    Grouping,
}

/// One open group (V8 `ParserState`).
#[derive(Debug)]
struct ParserState {
    builder: Builder,
    group_type: SubexpressionType,
    lookaround_type: LookaroundType,
    /// The capture index of this group (Capture), else the captures started before it.
    capture_index: u32,
    capture_name: Option<Vec<u16>>,
    /// Closed intervals of capture indices that cannot take part here (they are in another
    /// alternative), increasing.
    non_participating: Vec<(u32, u32)>,
}

impl ParserState {
    fn new_alternative(&mut self, captures_started: u32) {
        // Nothing to do if no capture started before the alternative.
        if self.capture_index == captures_started {
            return;
        }
        // +1: capture_index is exclusive.
        let from = self.capture_index + 1;
        let to = captures_started;
        debug_assert!(from <= to);
        // `to` only grows (captures_started), so the new interval ends at or after the last
        // one. Merge it into the last one when they overlap or touch: the union is the same
        // set of indices, and the list stays disjoint and increasing. (V8 merges only an
        // interval that grows by exactly 1, so `(?<a>x)()|(?<a>x)()|...` piles up overlapping
        // intervals there: a cubic duplicate-name scan; M7-P4 fixer r2.)
        debug_assert!(
            self.non_participating
                .last()
                .is_none_or(|last| to >= last.1)
        );
        if let Some(last) = self.non_participating.last_mut()
            && from <= last.1 + 1
        {
            *last = (last.0.min(from), last.1.max(to));
        } else {
            // The list stays disjoint and increasing (the duplicate-name scan stops early).
            debug_assert!(
                self.non_participating
                    .last()
                    .is_none_or(|last| last.1 + 1 < from),
                "{:?} then ({from}, {to})",
                self.non_participating
            );
            self.non_participating.push((from, to));
        }
    }
}

// ---- class set results ---------------------------------------------------------------------

/// The production of a ClassSetOperand (V8 `ClassSetOperandType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClassSetOperandType {
    ClassSetCharacter,
    ClassStringDisjunction,
    NestedClass,
    CharacterClassEscape,
    ClassSetRange,
}

/// V8 `ClassSetResult<kBuildAST>` (success only; failure is `Err`).
#[derive(Debug)]
struct ClassSetResult {
    tree: Option<Tree>,
    may_contain_strings: bool,
}

impl ClassSetResult {
    fn new(tree: Option<Tree>, may_contain_strings: bool) -> ClassSetResult {
        ClassSetResult {
            tree,
            may_contain_strings,
        }
    }
}

fn may_contain_strings(ty: ClassSetOperandType, operand: &ClassSetResult) -> bool {
    match ty {
        ClassSetOperandType::ClassSetCharacter | ClassSetOperandType::ClassSetRange => false,
        ClassSetOperandType::CharacterClassEscape
        | ClassSetOperandType::ClassStringDisjunction
        | ClassSetOperandType::NestedClass => operand.may_contain_strings,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InClassEscapeState {
    InClass,
    NotInClass,
}

// ---- Parser --------------------------------------------------------------------------------

/// The parser (V8 `ParserImpl`).
pub(crate) struct Parser<'a, T: Unit> {
    input: &'a [T],
    current: u32,
    flags: Flags,
    /// Act as if `u` were set (inside a group name).
    force_unicode: bool,
    /// May be one past the end, or more (V8 int arithmetic), so signed.
    next_pos: isize,
    captures_started: u32,
    /// Only valid after the scan for captures.
    capture_count: u32,
    quantifier_count: u32,
    lookaround_count: u32,
    has_more: bool,
    contains_anchor: bool,
    is_scanned_for_captures: bool,
    has_named_captures: bool,
    failed: bool,
    error: Option<Error>,
    error_pos: usize,
    /// The names of the captures, by index - 1 (V8 `captures_`, the name part).
    capture_names: Vec<Option<Vec<u16>>>,
    /// Name -> the indices of the groups with that name (V8 `named_captures_`).
    named_captures: Option<BTreeMap<Vec<u16>, Vec<u32>>>,
    /// The names of `\k<name>` references, in order (V8 `named_back_references_`).
    named_back_references: Vec<Vec<u16>>,
    /// The open groups; the last one is the current state.
    states: Vec<ParserState>,
    /// The depth of nested `v`-mode classes being parsed.
    class_depth: usize,
}

impl<'a, T: Unit> Parser<'a, T> {
    pub(crate) fn new(input: &'a [T], flags: Flags) -> Parser<'a, T> {
        let mut p = Parser {
            input,
            current: END,
            flags,
            force_unicode: false,
            next_pos: 0,
            captures_started: 0,
            capture_count: 0,
            quantifier_count: 0,
            lookaround_count: 0,
            has_more: true,
            contains_anchor: false,
            is_scanned_for_captures: false,
            has_named_captures: false,
            failed: false,
            error: None,
            error_pos: 0,
            capture_names: Vec::new(),
            named_captures: None,
            named_back_references: Vec::new(),
            states: Vec::new(),
            class_depth: 0,
        };
        p.advance();
        p
    }

    fn input_length(&self) -> isize {
        self.input.len() as isize
    }

    fn input_at(&self, index: isize) -> u16 {
        self.input[index as usize].unit()
    }

    fn current(&self) -> u32 {
        self.current
    }

    fn has_more(&self) -> bool {
        self.has_more
    }

    fn has_next(&self) -> bool {
        self.next_pos < self.input_length()
    }

    fn is_unicode_mode(&self) -> bool {
        self.flags.either_unicode() || self.force_unicode
    }

    fn unicode_sets(&self) -> bool {
        self.flags.unicode_sets()
    }

    /// V8 `position()`: the index of `current`.
    fn position(&self) -> isize {
        let current_is_surrogate = self.current != END && self.current > 0xffff;
        self.next_pos - if current_is_surrogate { 2 } else { 1 }
    }

    fn read_next(&mut self, update_position: bool) -> u32 {
        let mut position = self.next_pos;
        let c0 = u32::from(self.input_at(position));
        position += 1;
        let mut res = c0;
        if T::WIDE && self.is_unicode_mode() && position < self.input_length() && is_lead(c0) {
            let c1 = u32::from(self.input_at(position));
            if is_trail(c1) {
                res = combine(c0, c1);
                position += 1;
            }
        }
        if update_position {
            self.next_pos = position;
        }
        res
    }

    fn next(&mut self) -> u32 {
        if self.has_next() {
            self.read_next(false)
        } else {
            END
        }
    }

    fn advance(&mut self) {
        if self.has_next() {
            self.current = self.read_next(true);
        } else {
            self.current = END;
            // So that position() is 1 after the last character, which Reset() needs.
            self.next_pos = self.input_length() + 1;
            self.has_more = false;
        }
    }

    fn advance_by(&mut self, dist: isize) {
        self.next_pos += dist - 1;
        self.advance();
    }

    /// Rewinds to before the previous `advance` (one code point).
    fn rewind_by_one_codepoint(&mut self) {
        if !self.has_more() {
            return;
        }
        let rewind_by = if self.current > 0xffff { -2 } else { -1 };
        self.advance_by(rewind_by);
    }

    fn reset(&mut self, pos: isize) {
        self.next_pos = pos;
        self.has_more = pos < self.input_length();
        self.advance();
    }

    fn report_error(&mut self, error: Error) -> Failed {
        if !self.failed {
            self.failed = true;
            self.error = Some(error);
            self.error_pos = self.position().max(0) as usize;
            // Zip to the end so no more input is read.
            self.current = END;
            self.next_pos = self.input_length();
            self.has_more = false;
        }
        Failed
    }

    fn is_syntax_character_or_slash(c: u32) -> bool {
        matches!(
            char::from_u32(c),
            Some(
                '^' | '$'
                    | '\\'
                    | '.'
                    | '*'
                    | '+'
                    | '?'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '|'
                    | '/'
            )
        )
    }

    fn is_class_set_syntax_character(c: u32) -> bool {
        matches!(
            char::from_u32(c),
            Some('(' | ')' | '[' | ']' | '{' | '}' | '/' | '-' | '\\' | '|')
        )
    }

    fn is_class_set_reserved_punctuator(c: u32) -> bool {
        matches!(
            char::from_u32(c),
            Some('&' | '-' | '!' | '#' | '%' | ',' | ':' | ';' | '<' | '=' | '>' | '@' | '`' | '~')
        )
    }

    fn is_class_set_reserved_double_punctuator(&mut self, c: u32) -> bool {
        let double = matches!(
            char::from_u32(c),
            Some(
                '&' | '!'
                    | '#'
                    | '$'
                    | '%'
                    | '*'
                    | '+'
                    | ','
                    | '.'
                    | ':'
                    | ';'
                    | '<'
                    | '='
                    | '>'
                    | '?'
                    | '@'
                    | '^'
                    | '`'
                    | '~'
            )
        );
        double && self.next() == c
    }

    fn state(&mut self) -> &mut ParserState {
        self.states
            .last_mut()
            .expect("the initial state is always there")
    }

    fn builder(&mut self) -> &mut Builder {
        &mut self.state().builder
    }

    // ---- Pattern, Disjunction --------------------------------------------------------------

    /// Pattern :: Disjunction.
    fn parse_pattern(&mut self) -> PResult<Tree> {
        let mut result = self.parse_disjunction()?;
        self.patch_named_back_references(&mut result)?;
        debug_assert!(!self.has_more());
        Ok(result)
    }

    fn push_state(
        &mut self,
        group_type: SubexpressionType,
        lookaround_type: LookaroundType,
        capture_index: u32,
        capture_name: Option<Vec<u16>>,
        flags: Flags,
    ) -> PResult<()> {
        if self.states.len() + self.class_depth > MAX_NESTING {
            return Err(self.report_error(Error::StackOverflow));
        }
        let non_participating = self
            .states
            .last()
            .map(|s| s.non_participating.clone())
            .unwrap_or_default();
        self.states.push(ParserState {
            builder: Builder::new(flags),
            group_type,
            lookaround_type,
            capture_index,
            capture_name,
            non_participating,
        });
        Ok(())
    }

    /// Disjunction, Alternative, Term (V8 `ParseDisjunction`).
    fn parse_disjunction(&mut self) -> PResult<Tree> {
        self.push_state(
            SubexpressionType::Initial,
            LookaroundType::Lookahead,
            0,
            None,
            self.flags,
        )?;
        loop {
            match self.current() {
                END => {
                    if self.failed {
                        return Err(Failed);
                    }
                    if self.states.len() > 1 {
                        // Inside a group at the end of the input.
                        return Err(self.report_error(Error::UnterminatedGroup));
                    }
                    debug_assert_eq!(self.states[0].group_type, SubexpressionType::Initial);
                    return Ok(self.builder().take_regexp());
                }
                c if c == ch(b')') => {
                    if self.states.len() <= 1 {
                        return Err(self.report_error(Error::UnmatchedParen));
                    }
                    self.advance();
                    // End this disjunction; its content becomes one atom.
                    let mut body = self.builder().take_regexp();
                    let end_capture_index = self.captures_started;
                    let state = self.states.last().unwrap();
                    let capture_index = state.capture_index;
                    let group_type = state.group_type;
                    let lookaround_type = state.lookaround_type;
                    let group_flags = state.builder.flags;
                    match group_type {
                        SubexpressionType::Capture => {
                            let name = state.capture_name.clone();
                            if name.is_some() {
                                self.create_named_capture_at_index(capture_index)?;
                            }
                            body = Tree::Capture(Capture {
                                index: capture_index,
                                name,
                                body: Box::new(body),
                            });
                        }
                        SubexpressionType::Grouping => {
                            body = Tree::Group(Group {
                                flags: group_flags,
                                body: Box::new(body),
                            });
                        }
                        SubexpressionType::PositiveLookaround
                        | SubexpressionType::NegativeLookaround => {
                            body = Tree::Lookaround(Lookaround {
                                positive: group_type == SubexpressionType::PositiveLookaround,
                                kind: lookaround_type,
                                capture_count: end_capture_index - capture_index,
                                capture_from: capture_index,
                                index: self.lookaround_count,
                                body: Box::new(body),
                            });
                            self.lookaround_count += 1;
                        }
                        SubexpressionType::Initial => {
                            unreachable!("the initial state is never closed")
                        }
                    }
                    // Restore the previous state.
                    self.states.pop();
                    self.flags = self.builder().flags;
                    self.builder().add_atom(body);
                    // For compatibility with JSC and ES3, quantifiers after lookaheads are
                    // allowed (checked below).
                }
                c if c == ch(b'|') => {
                    self.advance();
                    let started = self.captures_started;
                    self.state().new_alternative(started);
                    self.builder().new_alternative();
                    continue;
                }
                c if c == ch(b'*') || c == ch(b'+') || c == ch(b'?') => {
                    return Err(self.report_error(Error::NothingToRepeat));
                }
                c if c == ch(b'^') => {
                    self.advance();
                    let ty = if self.builder().flags.multiline() {
                        AssertionType::StartOfLine
                    } else {
                        AssertionType::StartOfInput
                    };
                    self.builder().add_assertion(Tree::Assertion(ty));
                    self.contains_anchor = true;
                    continue;
                }
                c if c == ch(b'$') => {
                    self.advance();
                    let ty = if self.builder().flags.multiline() {
                        AssertionType::EndOfLine
                    } else {
                        AssertionType::EndOfInput
                    };
                    self.builder().add_assertion(Tree::Assertion(ty));
                    continue;
                }
                c if c == ch(b'.') => {
                    self.advance();
                    let set = if self.builder().flags.dot_all() {
                        StandardSet::Everything
                    } else {
                        StandardSet::NotLineTerminator
                    };
                    self.builder().add_class_ranges(ClassRanges {
                        items: vec![ClassItem::Standard(set)],
                        negated: false,
                    });
                }
                c if c == ch(b'(') => {
                    self.parse_open_parenthesis()?;
                    self.flags = self.builder().flags;
                    continue;
                }
                c if c == ch(b'[') => {
                    let cc = self.parse_character_class()?;
                    match cc.tree.expect("a class always has a tree") {
                        Tree::ClassRanges(cr) => self.builder().add_class_ranges(cr),
                        t => {
                            debug_assert!(matches!(t, Tree::ClassSetExpression(_)));
                            self.builder().add_term(t);
                        }
                    }
                }
                // Atom :: \ AtomEscape
                c if c == ch(b'\\') => {
                    if self.parse_atom_escape()? {
                        continue;
                    }
                }
                c if c == ch(b'{') => {
                    if self.parse_interval_quantifier().is_some() {
                        return Err(self.report_error(Error::NothingToRepeat));
                    }
                    self.lone_quantifier_bracket()?;
                }
                c if c == ch(b'}') || c == ch(b']') => {
                    self.lone_quantifier_bracket()?;
                }
                c => {
                    self.builder().add_unicode_character(c);
                    self.advance();
                }
            }
            self.parse_quantifier()?;
        }
    }

    /// `{`, `}` or `]` as a literal: an error in Unicode mode.
    fn lone_quantifier_bracket(&mut self) -> PResult<()> {
        if self.is_unicode_mode() {
            return Err(self.report_error(Error::LoneQuantifierBrackets));
        }
        let c = self.current();
        self.builder().add_unicode_character(c);
        self.advance();
        Ok(())
    }

    /// The quantifier after an atom, if any (the second switch of V8 `ParseDisjunction`).
    fn parse_quantifier(&mut self) -> PResult<()> {
        let (min, max) = match char::from_u32(self.current()) {
            Some('*') => {
                self.advance();
                (0, INFINITY)
            }
            Some('+') => {
                self.advance();
                (1, INFINITY)
            }
            Some('?') => {
                self.advance();
                (0, 1)
            }
            Some('{') => match self.parse_interval_quantifier() {
                Some((min, max, out_of_order)) => {
                    if out_of_order {
                        return Err(self.report_error(Error::RangeOutOfOrder));
                    }
                    (min, max)
                }
                None => {
                    if self.is_unicode_mode() {
                        // Incomplete quantifiers are not allowed.
                        return Err(self.report_error(Error::IncompleteQuantifier));
                    }
                    return Ok(());
                }
            },
            _ => return Ok(()),
        };
        let mut kind = QuantifierType::Greedy;
        if self.current() == ch(b'?') {
            kind = QuantifierType::NonGreedy;
            self.advance();
        }
        let index = self.quantifier_count;
        if !self.builder().add_quantifier_to_atom(min, max, index, kind) {
            return Err(self.report_error(Error::InvalidQuantifier));
        }
        self.quantifier_count += 1;
        Ok(())
    }

    /// `\` AtomEscape at `current` (the `'\\'` case of V8 `ParseDisjunction`). `Ok(true)` for
    /// an assertion (no quantifier check follows: V8 `continue`).
    fn parse_atom_escape(&mut self) -> PResult<bool> {
        let escaped_char = self.next();
        match escaped_char {
            END => return Err(self.report_error(Error::EscapeAtEndOfPattern)),
            // [+UnicodeMode] DecimalEscape; [~UnicodeMode] DecimalEscape only up to the
            // number of capturing groups; else CharacterEscape (legacy octal, \8 \9).
            c if (ch(b'1')..=ch(b'9')).contains(&c) => {
                if let Some(index) = self.parse_back_reference_index() {
                    if self.is_inside_capture_group_index(index) {
                        // A back reference inside the group it refers to: nothing can be
                        // captured yet, so it is empty.
                        self.builder().add_empty();
                    } else {
                        self.builder().add_atom(Tree::BackReference(BackReference {
                            captures: vec![index],
                            name: None,
                        }));
                    }
                    return Ok(false);
                }
                // With /u and /v, no identity escapes except syntax characters.
                if self.is_unicode_mode() {
                    return Err(self.report_error(Error::InvalidEscape));
                }
                let first_digit = self.next();
                if first_digit == ch(b'8') || first_digit == ch(b'9') {
                    self.builder().add_character(first_digit as u16);
                    self.advance_by(2);
                    return Ok(false);
                }
                self.parse_decimal_zero_escape()?;
            }
            c if c == ch(b'0') => self.parse_decimal_zero_escape()?,
            c if c == ch(b'b') || c == ch(b'B') => {
                self.advance_by(2);
                let ty = if c == ch(b'b') {
                    AssertionType::Boundary
                } else {
                    AssertionType::NonBoundary
                };
                self.builder().add_assertion(Tree::Assertion(ty));
                return Ok(true);
            }
            // AtomEscape :: CharacterClassEscape
            c if StandardSet::of_escape(c).is_some() => {
                let mut items = Vec::with_capacity(1);
                let parsed = self.try_parse_character_class_escape(
                    c,
                    InClassEscapeState::NotInClass,
                    &mut items,
                    None,
                )?;
                debug_assert!(parsed.is_some());
                self.builder().add_class_ranges(ClassRanges {
                    items,
                    negated: false,
                });
            }
            c if c == ch(b'p') || c == ch(b'P') => {
                let mut items = Vec::with_capacity(1);
                let mut strings = ClassStrings::new();
                let parsed = self.try_parse_character_class_escape(
                    c,
                    InClassEscapeState::NotInClass,
                    &mut items,
                    Some(&mut strings),
                )?;
                if parsed.is_some() {
                    let term = if self.unicode_sets() {
                        Tree::ClassSetOperand(ClassSetOperand { items, strings })
                    } else {
                        Tree::ClassRanges(ClassRanges {
                            items,
                            negated: false,
                        })
                    };
                    self.builder().add_term(term);
                } else {
                    assert!(!self.is_unicode_mode());
                    self.advance_by(2);
                    self.builder().add_character(c as u16); // IdentityEscape.
                }
            }
            // AtomEscape :: k GroupName. An identity escape for non-Unicode patterns without
            // named groups, else a named back reference. As V8, the scan for named groups
            // comes first, in every mode.
            c if c == ch(b'k') => {
                let has_named_captures = self.has_named_captures(InClassEscapeState::NotInClass);
                if self.is_unicode_mode() || has_named_captures {
                    self.advance_by(2);
                    self.parse_named_back_reference()?;
                } else {
                    self.parse_character_escape_atom()?;
                }
            }
            // AtomEscape :: CharacterEscape
            _ => self.parse_character_escape_atom()?,
        }
        Ok(false)
    }

    /// AtomEscape :: CharacterEscape (the `default` case of V8's AtomEscape switch).
    fn parse_character_escape_atom(&mut self) -> PResult<()> {
        let c = self.parse_character_escape(InClassEscapeState::NotInClass)?;
        self.builder().add_unicode_character(c);
        Ok(())
    }

    /// The `'0'` case of V8's AtomEscape switch (also reached from `\1`-`\7` that are not
    /// back references): `\0`, or a legacy octal escape.
    fn parse_decimal_zero_escape(&mut self) -> PResult<()> {
        self.advance();
        if self.is_unicode_mode() && is_decimal_digit(self.next()) {
            // A decimal escape with a leading 0 is not octal.
            return Err(self.report_error(Error::InvalidDecimalEscape));
        }
        let octal = self.parse_octal_literal();
        self.builder().add_character(octal as u16);
        Ok(())
    }

    // ---- groups ----------------------------------------------------------------------------

    fn parse_open_parenthesis(&mut self) -> PResult<()> {
        let state = self.states.last().unwrap();
        let mut lookaround_type = state.lookaround_type;
        let mut flags = state.builder.flags;
        let mut is_named_capture = false;
        let mut capture_name = None;
        let mut subexpr_type = SubexpressionType::Capture;
        let mut parsing_modifiers = false;
        let mut modifiers_polarity = true;
        let mut modifiers = Flags::empty();
        self.advance();
        if self.current() == ch(b'?') {
            loop {
                let next = self.next();
                match char::from_u32(next) {
                    Some('-') => {
                        self.advance();
                        parsing_modifiers = true;
                        if !modifiers_polarity {
                            return Err(self.report_error(Error::MultipleFlagDashes));
                        }
                        modifiers_polarity = false;
                    }
                    Some('m' | 'i' | 's') => {
                        self.advance();
                        parsing_modifiers = true;
                        let flag = Flags::from_letter(next).unwrap();
                        if modifiers.contains(flag) {
                            return Err(self.report_error(Error::RepeatedFlag));
                        }
                        modifiers = modifiers.union(flag);
                        flags = flags.with(flag, modifiers_polarity);
                    }
                    Some(':') => {
                        self.advance_by(2);
                        parsing_modifiers = false;
                        subexpr_type = SubexpressionType::Grouping;
                    }
                    Some('=' | '!') => {
                        self.advance_by(2);
                        if parsing_modifiers {
                            return Err(self.report_error(Error::InvalidGroup));
                        }
                        lookaround_type = LookaroundType::Lookahead;
                        subexpr_type = if next == ch(b'=') {
                            SubexpressionType::PositiveLookaround
                        } else {
                            SubexpressionType::NegativeLookaround
                        };
                    }
                    Some('<') => {
                        self.advance();
                        if parsing_modifiers {
                            return Err(self.report_error(Error::InvalidGroup));
                        }
                        let n = self.next();
                        if n == ch(b'=') || n == ch(b'!') {
                            self.advance_by(2);
                            lookaround_type = LookaroundType::Lookbehind;
                            subexpr_type = if n == ch(b'=') {
                                SubexpressionType::PositiveLookaround
                            } else {
                                SubexpressionType::NegativeLookaround
                            };
                        } else {
                            is_named_capture = true;
                            self.has_named_captures = true;
                            self.advance();
                        }
                    }
                    _ => return Err(self.report_error(Error::InvalidGroup)),
                }
                if !parsing_modifiers {
                    break;
                }
            }
        }
        // A dash with no modifier: (?-:...).
        if !modifiers_polarity && modifiers == Flags::empty() {
            return Err(self.report_error(Error::InvalidFlagGroup));
        }
        if subexpr_type == SubexpressionType::Capture {
            if self.captures_started >= MAX_CAPTURES {
                return Err(self.report_error(Error::TooManyCaptures));
            }
            self.captures_started += 1;
            if is_named_capture {
                capture_name = Some(self.parse_capture_group_name()?);
            }
        }
        // Store the current state and start a new disjunction.
        let started = self.captures_started;
        self.push_state(subexpr_type, lookaround_type, started, capture_name, flags)
    }

    /// The mini-parser that counts the capturing groups of the whole pattern (V8
    /// `ScanForCaptures`): it skips classes and escapes and tells `(?<name>` from `(?<=`.
    fn scan_for_captures(&mut self, in_class_escape_state: InClassEscapeState) {
        debug_assert!(!self.is_scanned_for_captures);
        let saved_position = self.position();
        // Start with the captures started before this position.
        let mut capture_count = self.captures_started;
        // Inside a class, skip to its end first.
        if in_class_escape_state == InClassEscapeState::InClass {
            debug_assert!(!self.is_unicode_mode());
            loop {
                let c = self.current();
                if c == END {
                    break;
                }
                self.advance();
                if c == ch(b'\\') {
                    self.advance();
                } else if c == ch(b']') {
                    break;
                }
            }
        }
        // Count the captures after this position.
        loop {
            let n = self.current();
            if n == END {
                break;
            }
            self.advance();
            if n == ch(b'\\') {
                self.advance();
            } else if n == ch(b'[') {
                let mut class_nest_level = 0u32;
                loop {
                    let c = self.current();
                    if c == END {
                        break;
                    }
                    self.advance();
                    if c == ch(b'\\') {
                        self.advance();
                    } else if c == ch(b'[') {
                        // With /v, '[' inside a class opens a nested class.
                        if self.unicode_sets() {
                            class_nest_level += 1;
                        }
                    } else if c == ch(b']') {
                        if class_nest_level == 0 {
                            break;
                        }
                        class_nest_level -= 1;
                    }
                }
            } else if n == ch(b'(') {
                if self.current() == ch(b'?') {
                    // (?: (?<= (?<! are not captures; (?< is (a possible) named one.
                    self.advance();
                    if self.current() != ch(b'<') {
                        continue;
                    }
                    self.advance();
                    if self.current() == ch(b'=') || self.current() == ch(b'!') {
                        continue;
                    }
                    self.has_named_captures = true;
                }
                capture_count += 1;
            }
        }
        self.capture_count = capture_count;
        self.is_scanned_for_captures = true;
        self.reset(saved_position);
    }

    /// `\` DecimalEscape as a back reference index, if the number is a group number (V8
    /// `ParseBackReferenceIndex`). Otherwise the position is restored.
    fn parse_back_reference_index(&mut self) -> Option<u32> {
        debug_assert_eq!(self.current(), ch(b'\\'));
        let start = self.position();
        let mut value = self.next() - ch(b'0');
        self.advance_by(2);
        loop {
            let c = self.current();
            if !is_decimal_digit(c) {
                break;
            }
            value = 10 * value + (c - ch(b'0'));
            if value > MAX_CAPTURES {
                self.reset(start);
                return None;
            }
            self.advance();
        }
        if value > self.captures_started {
            if !self.is_scanned_for_captures {
                self.scan_for_captures(InClassEscapeState::NotInClass);
            }
            if value > self.capture_count {
                self.reset(start);
                return None;
            }
        }
        Some(value)
    }

    /// RegExpIdentifierName of a group (V8 `ParseCaptureGroupName`). `current` is the first
    /// character of the name; at the end it is the character after `>`.
    fn parse_capture_group_name(&mut self) -> PResult<Vec<u16>> {
        // Rewind by one so that a name that starts with a surrogate pair reads as one code
        // point also without `u`.
        self.rewind_by_one_codepoint();
        let mut name = Vec::new();
        // RegExpIdentifierName is [+UnicodeMode]: surrogate pairs and \u{...} in any mode.
        debug_assert!(!self.force_unicode);
        self.force_unicode = true;
        let r = self.parse_capture_group_name_unicode(&mut name);
        self.force_unicode = false;
        r?;
        // Back to pointing at the next character, as the rest of the parser expects.
        self.advance();
        Ok(name)
    }

    fn parse_capture_group_name_unicode(&mut self, name: &mut Vec<u16>) -> PResult<()> {
        let mut at_start = true;
        loop {
            self.advance();
            let mut c = self.current();
            // Convert unicode escapes.
            let escaped = c == ch(b'\\') && self.next() == ch(b'u');
            if escaped {
                self.advance_by(2);
                match self.parse_unicode_escape() {
                    Some(v) => c = v,
                    None => return Err(self.report_error(Error::InvalidUnicodeEscape)),
                }
                self.rewind_by_one_codepoint();
            }
            // V8's identifier tables also classify the backslash as a name character.
            if c == ch(b'\\') {
                return Err(self.report_error(Error::InvalidCaptureGroupName));
            }
            if at_start {
                if !unicode::is_identifier_start(c) {
                    return Err(self.report_error(Error::InvalidCaptureGroupName));
                }
                push_code_unit(name, c);
                at_start = false;
            } else if c == ch(b'>') && !escaped {
                // Only a source `>` ends the name (ECMA-262 22.2.1 GroupName); an escape of
                // `>` is a name character that is not an IdentifierPartChar (22.2.1.1), so the
                // last branch refuses it. V8 ends the name there (M7-P4 fixer r2, R6).
                return Ok(());
            } else if unicode::is_identifier_part(c) {
                push_code_unit(name, c);
            } else {
                return Err(self.report_error(Error::InvalidCaptureGroupName));
            }
        }
    }

    fn is_inside_capture_group_index(&self, index: u32) -> bool {
        for s in self.states.iter().rev() {
            if s.group_type != SubexpressionType::Capture {
                continue;
            }
            // The matching capture index.
            if index == s.capture_index {
                return true;
            }
            // An index above what has been parsed up to this state.
            if index > s.capture_index {
                return false;
            }
        }
        false
    }

    fn is_inside_capture_group_name(states: &[ParserState], name: &[u16]) -> bool {
        states
            .iter()
            .rev()
            .any(|s| s.capture_name.as_deref() == Some(name))
    }

    /// V8 `GetCapture`: makes the capture list long enough and names the capture.
    fn set_capture_name(&mut self, index: u32, name: Vec<u16>) {
        let known = if self.is_scanned_for_captures {
            self.capture_count
        } else {
            self.captures_started
        };
        assert!(index >= 1 && index <= known, "capture index out of range");
        if self.capture_names.len() < known as usize {
            self.capture_names.resize(known as usize, None);
        }
        let slot = &mut self.capture_names[index as usize - 1];
        debug_assert!(slot.is_none());
        *slot = Some(name);
    }

    /// Registers the named capture of the current state at `index` (V8
    /// `CreateNamedCaptureAtIndex`). Fails on a duplicate name that can take part in the same
    /// match.
    fn create_named_capture_at_index(&mut self, index: u32) -> PResult<()> {
        let state = self.states.last().unwrap();
        let name = state.capture_name.clone().expect("a named capture");
        debug_assert!(0 < index && index <= self.captures_started);
        let duplicate = match &self.named_captures {
            None => false,
            Some(map) => match map.get(&name) {
                None => false,
                Some(indices) => indices.iter().any(|&named_index| {
                    let mut is_duplicate = true;
                    for &(from, to) in &state.non_participating {
                        #[cfg(test)]
                        INTERVAL_STEPS.with(|n| n.set(n.get() + 1));
                        // Inside one non-participating interval: not a duplicate.
                        if (from..=to).contains(&named_index) {
                            is_duplicate = false;
                            break;
                        }
                        // Intervals increase, so stop past the index.
                        if named_index <= from {
                            break;
                        }
                    }
                    is_duplicate
                }),
            },
        };
        if duplicate {
            return Err(self.report_error(Error::DuplicateCaptureGroupName));
        }
        // A nested group of the same name in the same alternative.
        let parents = &self.states[..self.states.len() - 1];
        if Self::is_inside_capture_group_name(parents, &name) {
            return Err(self.report_error(Error::DuplicateCaptureGroupName));
        }
        self.set_capture_name(index, name.clone());
        self.named_captures
            .get_or_insert_with(BTreeMap::new)
            .entry(name)
            .or_default()
            .push(index);
        Ok(())
    }

    /// `\k<name>` after `\k` (V8 `ParseNamedBackReference`).
    fn parse_named_back_reference(&mut self) -> PResult<()> {
        if self.current() != ch(b'<') {
            return Err(self.report_error(Error::InvalidNamedReference));
        }
        self.advance();
        let name = self.parse_capture_group_name()?;
        if Self::is_inside_capture_group_name(&self.states, &name) {
            self.builder().add_empty();
        } else {
            self.builder().add_atom(Tree::BackReference(BackReference {
                captures: Vec::new(),
                name: Some(name.clone()),
            }));
            self.named_back_references.push(name);
        }
        Ok(())
    }

    /// Checks that every `\k<name>` names a group and fills in its group indices (V8
    /// `PatchNamedBackReferences`; V8 patches pointers, here the tree is walked).
    fn patch_named_back_references(&mut self, tree: &mut Tree) -> PResult<()> {
        if self.named_back_references.is_empty() {
            return Ok(());
        }
        let Some(map) = self.named_captures.take() else {
            return Err(self.report_error(Error::InvalidNamedCaptureReference));
        };
        if self
            .named_back_references
            .iter()
            .any(|name| !map.contains_key(name))
        {
            self.named_captures = Some(map);
            return Err(self.report_error(Error::InvalidNamedCaptureReference));
        }
        fill_named_references(tree, &map);
        self.named_captures = Some(map);
        Ok(())
    }

    /// True if the pattern has a named group; may scan ahead (V8 `HasNamedCaptures`).
    fn has_named_captures(&mut self, in_class_escape_state: InClassEscapeState) -> bool {
        if self.has_named_captures || self.is_scanned_for_captures {
            return self.has_named_captures;
        }
        self.scan_for_captures(in_class_escape_state);
        self.has_named_captures
    }

    // ---- quantifiers and escapes -----------------------------------------------------------

    /// `{n}`, `{n,}`, `{n,m}` at `current` (V8 `ParseIntervalQuantifier`): (min, max, whether
    /// the MV of n is larger than the MV of m). `None` restores the position. Values above
    /// [`INFINITY`] become [`INFINITY`]; the order test reads the digits, not the capped
    /// values (ECMA-262 22.2.1.1; V8 compares the capped values, so it takes
    /// `{2147483649,2147483648}`; M7-P4 fixer r2, R7).
    fn parse_interval_quantifier(&mut self) -> Option<(u32, u32, bool)> {
        debug_assert_eq!(self.current(), ch(b'{'));
        let start = self.position();
        self.advance();
        if !is_decimal_digit(self.current()) {
            self.reset(start);
            return None;
        }
        let (min, min_digits) = self.parse_quantifier_digits();
        let max;
        let mut out_of_order = false;
        if self.current() == ch(b'}') {
            max = min;
            self.advance();
        } else if self.current() == ch(b',') {
            self.advance();
            if self.current() == ch(b'}') {
                max = INFINITY;
                self.advance();
            } else {
                let max_digits;
                (max, max_digits) = self.parse_quantifier_digits();
                if self.current() != ch(b'}') {
                    self.reset(start);
                    return None;
                }
                out_of_order = self.compare_digits(min_digits, max_digits).is_gt();
                self.advance();
            }
        } else {
            self.reset(start);
            return None;
        }
        Some((min, max, out_of_order))
    }

    /// Compares the MVs of two DecimalDigits by their input spans (ASCII digits, one code unit
    /// each): with the leading zeros dropped, the longer one is larger; at equal length the
    /// text decides.
    fn compare_digits(&self, a: (isize, isize), b: (isize, isize)) -> core::cmp::Ordering {
        let significant = |(mut from, to): (isize, isize)| {
            while from + 1 < to && self.input_at(from) == u16::from(b'0') {
                from += 1;
            }
            (from, to)
        };
        let (a, b) = (significant(a), significant(b));
        (a.1 - a.0).cmp(&(b.1 - b.0)).then_with(|| {
            (a.0..a.1)
                .map(|i| self.input_at(i))
                .cmp((b.0..b.1).map(|i| self.input_at(i)))
        })
    }

    /// DecimalDigits of a quantifier, capped at [`INFINITY`] (V8's overflow loop), and the
    /// input span of the digits.
    fn parse_quantifier_digits(&mut self) -> (u32, (isize, isize)) {
        let from = self.position();
        let value = self.parse_quantifier_value();
        (value, (from, self.position()))
    }

    fn parse_quantifier_value(&mut self) -> u32 {
        let mut value = 0u32;
        while is_decimal_digit(self.current()) {
            let next = self.current() - ch(b'0');
            if value > (INFINITY - next) / 10 {
                // Overflow: skip the other digits.
                loop {
                    self.advance();
                    if !is_decimal_digit(self.current()) {
                        break;
                    }
                }
                return INFINITY;
            }
            value = 10 * value + next;
            self.advance();
        }
        value
    }

    /// LegacyOctalEscapeSequence: up to three octal digits with a value below 256.
    fn parse_octal_literal(&mut self) -> u32 {
        let digit = |c: u32| (ch(b'0')..=ch(b'7')).contains(&c);
        debug_assert!(digit(self.current()) || !self.has_more());
        let mut value = self.current().wrapping_sub(ch(b'0'));
        self.advance();
        if digit(self.current()) {
            value = value * 8 + self.current() - ch(b'0');
            self.advance();
            if value < 32 && digit(self.current()) {
                value = value * 8 + self.current() - ch(b'0');
                self.advance();
            }
        }
        value
    }

    /// `length` hex digits at `current`, or `None` with the position restored.
    fn parse_hex_escape(&mut self, length: usize) -> Option<u32> {
        let start = self.position();
        let mut val = 0u32;
        for _ in 0..length {
            let Some(d) = hex_value(self.current()) else {
                self.reset(start);
                return None;
            };
            val = val * 16 + d;
            self.advance();
        }
        Some(val)
    }

    /// RegExpUnicodeEscapeSequence after `\u` (V8 `ParseUnicodeEscape`): `\uXXXX`, a pair of
    /// surrogate escapes in Unicode mode, or `\u{...}` in Unicode mode.
    fn parse_unicode_escape(&mut self) -> Option<u32> {
        if self.current() == ch(b'{') && self.is_unicode_mode() {
            let start = self.position();
            self.advance();
            if let Some(v) = self.parse_unlimited_length_hex_number(0x10ffff)
                && self.current() == ch(b'}')
            {
                self.advance();
                return Some(v);
            }
            self.reset(start);
            return None;
        }
        // \u without {, or \u{...} not allowed.
        let value = self.parse_hex_escape(4)?;
        if self.is_unicode_mode() && is_lead(value) && self.current() == ch(b'\\') {
            // Try to read a trail surrogate.
            let start = self.position();
            if self.next() == ch(b'u') {
                self.advance_by(2);
                if let Some(trail) = self.parse_hex_escape(4)
                    && is_trail(trail)
                {
                    return Some(combine(value, trail));
                }
            }
            self.reset(start);
        }
        Some(value)
    }

    fn parse_unlimited_length_hex_number(&mut self, max_value: u32) -> Option<u32> {
        let mut x = 0u32;
        let mut d = hex_value(self.current())?;
        loop {
            x = x * 16 + d;
            if x > max_value {
                return None;
            }
            self.advance();
            match hex_value(self.current()) {
                Some(n) => d = n,
                None => break,
            }
        }
        Some(x)
    }

    /// `\p{...}` after `\p` (V8 `ParsePropertyClassName`): name_1 and, after `=`, name_2.
    fn parse_property_class_name(&mut self) -> Option<(String, Option<String>)> {
        fn is_value_character(c: u32) -> bool {
            char::from_u32(c).is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        if self.current() != ch(b'{') {
            return None;
        }
        let mut name_1 = String::new();
        let mut name_2 = None;
        self.advance();
        while self.current() != ch(b'}') && self.current() != ch(b'=') {
            if !is_value_character(self.current()) || !self.has_next() {
                return None;
            }
            name_1.push(char::from_u32(self.current()).unwrap());
            self.advance();
        }
        if self.current() == ch(b'=') {
            let mut value = String::new();
            self.advance();
            while self.current() != ch(b'}') {
                if !is_value_character(self.current()) || !self.has_next() {
                    return None;
                }
                value.push(char::from_u32(self.current()).unwrap());
                self.advance();
            }
            name_2 = Some(value);
        }
        self.advance();
        Some((name_1, name_2))
    }

    /// The property of `\p{..}` / `\P{..}` (V8 `AddPropertyClassRange`), with
    /// may-contain-strings.
    fn property_class_range(
        &self,
        negate: bool,
        name_1: &str,
        name_2: Option<&str>,
    ) -> Option<(Property, bool)> {
        match name_2 {
            None => {
                let p = unicode::lone_property(name_1, self.unicode_sets())?;
                let strings = matches!(p, Property::OfStrings(_));
                // Negation of properties with strings is not allowed (MayContainStrings).
                if negate && strings {
                    return None;
                }
                Some((p, strings))
            }
            Some(value) => Some((unicode::property_with_value(name_1, value)?, false)),
        }
    }

    /// CharacterEscape at `current` = `\` (V8 `ParseCharacterEscape`).
    fn parse_character_escape(&mut self, in_class: InClassEscapeState) -> PResult<u32> {
        debug_assert_eq!(self.current(), ch(b'\\'));
        debug_assert!(self.has_next());
        self.advance();
        let c = self.current();
        match char::from_u32(c) {
            // ControlEscape :: one of f n r t v
            Some('f') => {
                self.advance();
                return Ok(0x0c);
            }
            Some('n') => {
                self.advance();
                return Ok(0x0a);
            }
            Some('r') => {
                self.advance();
                return Ok(0x0d);
            }
            Some('t') => {
                self.advance();
                return Ok(0x09);
            }
            Some('v') => {
                self.advance();
                return Ok(0x0b);
            }
            // c ControlLetter
            Some('c') => {
                let control_letter = self.next();
                let letter = control_letter & !(ch(b'A') ^ ch(b'a'));
                if (ch(b'A')..=ch(b'Z')).contains(&letter) {
                    self.advance_by(2);
                    // Control letters map to the ASCII controls 0x00-0x1F.
                    return Ok(control_letter & 0x1f);
                }
                if self.is_unicode_mode() {
                    // With /u and /v, invalid escapes are not identity escapes.
                    return Err(self.report_error(Error::InvalidUnicodeEscape));
                }
                if in_class == InClassEscapeState::InClass
                    && (is_decimal_digit(control_letter) || control_letter == ch(b'_'))
                {
                    // Annex B ClassControlLetter: digits and underscore too, without u/v.
                    self.advance_by(2);
                    return Ok(control_letter & 0x1f);
                }
                // As JSC: the backslash is a literal character, not an escape.
                return Ok(ch(b'\\'));
            }
            // 0 [lookahead ∉ DecimalDigit]; [~UnicodeMode] LegacyOctalEscapeSequence
            Some('0') if !is_decimal_digit(self.next()) => {
                // \0 is NUL if no digit follows.
                self.advance();
                return Ok(0);
            }
            Some('0'..='7') => {
                // A decimal escape that is not a back reference is a 1-3 digit octal code.
                if self.is_unicode_mode() {
                    return Err(self.report_error(Error::InvalidDecimalEscape));
                }
                return Ok(self.parse_octal_literal());
            }
            // HexEscapeSequence
            Some('x') => {
                self.advance();
                if let Some(v) = self.parse_hex_escape(2) {
                    return Ok(v);
                }
                if self.is_unicode_mode() {
                    return Err(self.report_error(Error::InvalidEscape));
                }
                // \x not followed by two hex digits: an identity escape.
                return Ok(ch(b'x'));
            }
            // RegExpUnicodeEscapeSequence [?UnicodeMode]
            Some('u') => {
                self.advance();
                if let Some(v) = self.parse_unicode_escape() {
                    return Ok(v);
                }
                if self.is_unicode_mode() {
                    return Err(self.report_error(Error::InvalidUnicodeEscape));
                }
                return Ok(ch(b'u'));
            }
            _ => {}
        }
        // IdentityEscape: with /u only syntax characters; with /v also the reserved
        // punctuators inside a class; without either everything but `c`, and `k` when there
        // are named groups.
        if self.unicode_sets()
            && in_class == InClassEscapeState::InClass
            && Self::is_class_set_reserved_punctuator(c)
        {
            self.advance();
            return Ok(c);
        }
        if self.is_unicode_mode() {
            if !Self::is_syntax_character_or_slash(c) {
                return Err(self.report_error(Error::InvalidEscape));
            }
            self.advance();
            return Ok(c);
        }
        if c == ch(b'c') {
            return Err(self.report_error(Error::InvalidEscape));
        }
        // Advance before has_named_captures so the scan does not start inside an escape.
        self.advance();
        if c == ch(b'k') && self.has_named_captures(in_class) {
            return Err(self.report_error(Error::InvalidEscape));
        }
        Ok(c)
    }

    // ---- classes without v -----------------------------------------------------------------

    /// ClassContents without `v` (V8 `ParseClassRanges`).
    fn parse_class_ranges(&mut self, items: &mut Vec<ClassItem>) -> PResult<()> {
        while self.has_more() && self.current() != ch(b']') {
            let (char_1, is_class_1) = self.parse_class_escape(items)?;
            if self.current() == ch(b'-') {
                self.advance();
                if !self.has_more() {
                    // The caller reports the unterminated class.
                    break;
                } else if self.current() == ch(b']') {
                    if !is_class_1 {
                        items.push(ClassItem::Range(char_1, char_1));
                    }
                    items.push(ClassItem::Range(ch(b'-'), ch(b'-')));
                    break;
                }
                let (char_2, is_class_2) = self.parse_class_escape(items)?;
                if is_class_1 || is_class_2 {
                    // A class escape at either end: the '-' is literal.
                    if self.is_unicode_mode() {
                        return Err(self.report_error(Error::InvalidCharacterClass));
                    }
                    if !is_class_1 {
                        items.push(ClassItem::Range(char_1, char_1));
                    }
                    items.push(ClassItem::Range(ch(b'-'), ch(b'-')));
                    if !is_class_2 {
                        items.push(ClassItem::Range(char_2, char_2));
                    }
                    continue;
                }
                if char_1 > char_2 {
                    return Err(self.report_error(Error::OutOfOrderCharacterClass));
                }
                items.push(ClassItem::Range(char_1, char_2));
            } else if !is_class_1 {
                items.push(ClassItem::Range(char_1, char_1));
            }
        }
        Ok(())
    }

    /// ClassAtom (V8 `ParseClassEscape`): a class escape is added to `items` and `true` is
    /// returned; else the character.
    fn parse_class_escape(&mut self, items: &mut Vec<ClassItem>) -> PResult<(u32, bool)> {
        if self.current() != ch(b'\\') {
            // Not a ClassEscape.
            let c = self.current();
            self.advance();
            return Ok((c, false));
        }
        let next = self.next();
        if next == ch(b'b') {
            self.advance_by(2);
            return Ok((0x08, false));
        }
        if next == ch(b'-') && self.is_unicode_mode() {
            self.advance_by(2);
            return Ok((next, false));
        }
        if next == END {
            return Err(self.report_error(Error::EscapeAtEndOfPattern));
        }
        if self
            .try_parse_character_class_escape(next, InClassEscapeState::InClass, items, None)?
            .is_some()
        {
            return Ok((0, true));
        }
        let c = self.parse_character_escape(InClassEscapeState::InClass)?;
        Ok((c, false))
    }

    /// CharacterClassEscape after `\` (V8 `TryParseCharacterClassEscape`). `Ok(None)`: not a
    /// class escape, nothing read.
    fn try_parse_character_class_escape(
        &mut self,
        next: u32,
        in_class: InClassEscapeState,
        items: &mut Vec<ClassItem>,
        _strings: Option<&mut ClassStrings>,
    ) -> PResult<Option<ClassSetResult>> {
        debug_assert_eq!(self.current(), ch(b'\\'));
        if let Some(set) = StandardSet::of_escape(next) {
            items.push(ClassItem::Standard(set));
            self.advance_by(2);
            return Ok(Some(ClassSetResult::new(None, false)));
        }
        if next != ch(b'p') && next != ch(b'P') {
            return Ok(None);
        }
        if !self.is_unicode_mode() {
            return Ok(None);
        }
        let negate = next == ch(b'P');
        self.advance_by(2);
        let error = if in_class == InClassEscapeState::InClass {
            Error::InvalidClassPropertyName
        } else {
            Error::InvalidPropertyName
        };
        let Some((name_1, name_2)) = self.parse_property_class_name() else {
            return Err(self.report_error(error));
        };
        let Some((property, strings)) =
            self.property_class_range(negate, &name_1, name_2.as_deref())
        else {
            return Err(self.report_error(error));
        };
        items.push(ClassItem::Property {
            property,
            negated: negate,
        });
        Ok(Some(ClassSetResult::new(None, strings)))
    }

    // ---- classes with v --------------------------------------------------------------------

    /// Adds a `\q{}` alternative: one code point to `items`, any other length to `strings`
    /// (V8 `AddClassString`). True if a string was added.
    fn add_class_string(
        normalized: Vec<u32>,
        tree: Tree,
        items: &mut Vec<ClassItem>,
        strings: &mut ClassStrings,
    ) -> bool {
        let is_string = normalized.len() != 1;
        if !is_string {
            items.push(ClassItem::Range(normalized[0], normalized[0]));
        } else {
            strings.entry(ClassString(normalized)).or_insert(tree);
        }
        is_string
    }

    /// ClassStringDisjunction `\q{...}` (V8 `ParseClassStringDisjunction`).
    fn parse_class_string_disjunction(
        &mut self,
        items: &mut Vec<ClassItem>,
        strings: &mut ClassStrings,
    ) -> PResult<ClassSetResult> {
        debug_assert!(self.unicode_sets());
        self.advance_by(2);
        if self.current() != ch(b'{') {
            // An identity escape of 'q' is not allowed in Unicode mode.
            return Err(self.report_error(Error::InvalidEscape));
        }
        self.advance();
        let mut string = Vec::new();
        let mut string_terms = Vec::new();
        let mut string_builder = TextBuilder::new(self.flags);
        let mut contains_strings = false;
        while self.has_more() && self.current() != ch(b'}') {
            if self.current() == ch(b'|') {
                let tree = string_builder.take_regexp(&mut string_terms);
                contains_strings |=
                    Self::add_class_string(std::mem::take(&mut string), tree, items, strings);
                string_terms.clear();
                self.advance();
            } else {
                // V8 folds `c` with u_foldCase under `i` (simple folding: one code point to
                // one, so the length stays). Not ported: the compiler folds (RX1-1).
                let c = self.parse_class_set_character()?;
                string.push(c);
                string_builder.add_unicode_character(&mut string_terms, c);
            }
        }
        let tree = string_builder.take_regexp(&mut string_terms);
        contains_strings |= Self::add_class_string(string, tree, items, strings);
        // A missing '}' is reported by parse_class_set_character (the class is unterminated).
        self.advance();
        Ok(ClassSetResult::new(None, contains_strings))
    }

    /// ClassSetOperand as a tree (V8 `ParseClassSetOperand`, the 2-argument form).
    fn parse_class_set_operand_tree(&mut self) -> PResult<(ClassSetResult, ClassSetOperandType)> {
        let mut items = Vec::new();
        let mut strings = ClassStrings::new();
        let (mut operand, ty, character) =
            self.parse_class_set_operand(&mut items, &mut strings)?;
        debug_assert_ne!(ty, ClassSetOperandType::ClassSetRange);
        if operand.tree.is_none() {
            if ty == ClassSetOperandType::ClassSetCharacter {
                items.push(ClassItem::Range(character, character));
            }
            operand.tree = Some(Tree::ClassSetOperand(ClassSetOperand { items, strings }));
        }
        Ok((operand, ty))
    }

    /// ClassSetOperand (V8 `ParseClassSetOperand`, the 5-argument form): a nested class gives
    /// a tree; a ClassSetCharacter gives the character; the other forms add to
    /// `items`/`strings`.
    fn parse_class_set_operand(
        &mut self,
        items: &mut Vec<ClassItem>,
        strings: &mut ClassStrings,
    ) -> PResult<(ClassSetResult, ClassSetOperandType, u32)> {
        debug_assert!(self.unicode_sets());
        let c = self.current();
        if c == ch(b'\\') {
            let next = self.next();
            if next == ch(b'q') {
                let r = self.parse_class_string_disjunction(items, strings)?;
                return Ok((r, ClassSetOperandType::ClassStringDisjunction, 0));
            }
            if let Some(r) = self.try_parse_character_class_escape(
                next,
                InClassEscapeState::InClass,
                items,
                Some(strings),
            )? {
                return Ok((r, ClassSetOperandType::CharacterClassEscape, 0));
            }
        }
        if c == ch(b'[') {
            let r = self.parse_character_class()?;
            return Ok((r, ClassSetOperandType::NestedClass, 0));
        }
        let c = self.parse_class_set_character()?;
        Ok((
            ClassSetResult::new(None, false),
            ClassSetOperandType::ClassSetCharacter,
            c,
        ))
    }

    /// ClassSetCharacter (V8 `ParseClassSetCharacter`).
    fn parse_class_set_character(&mut self) -> PResult<u32> {
        debug_assert!(self.unicode_sets());
        let c = self.current();
        if c == ch(b'\\') {
            let next = self.next();
            if next == ch(b'b') {
                self.advance_by(2);
                return Ok(0x08);
            }
            if next == END {
                return Err(self.report_error(Error::EscapeAtEndOfPattern));
            }
            return self.parse_character_escape(InClassEscapeState::InClass);
        }
        if Self::is_class_set_syntax_character(c) {
            return Err(self.report_error(Error::InvalidCharacterInClass));
        }
        if self.is_class_set_reserved_double_punctuator(c) {
            return Err(self.report_error(Error::InvalidClassSetOperation));
        }
        self.advance();
        Ok(c)
    }

    /// ClassUnion (V8 `ParseClassUnion`).
    fn parse_class_union(
        &mut self,
        is_negated: bool,
        first_operand: ClassSetResult,
        first_operand_type: ClassSetOperandType,
        mut items: Vec<ClassItem>,
        mut strings: ClassStrings,
        mut character: u32,
    ) -> PResult<ClassSetResult> {
        let mut operands = Vec::with_capacity(2);
        // Either the lhs was added to items/strings (no tree), or it is a tree (then
        // items/strings are empty).
        let mut may_contain = may_contain_strings(first_operand_type, &first_operand);
        if let Some(t) = first_operand.tree {
            operands.push(t);
        }
        let mut last_type = first_operand_type;
        while self.has_more() && self.current() != ch(b']') {
            if self.current() == ch(b'-') {
                // A mix of ClassSetRange and ClassSubtraction is not allowed.
                if self.next() == ch(b'-') {
                    return Err(self.report_error(Error::InvalidClassSetOperation));
                }
                self.advance();
                if !self.has_more() {
                    // Reported below.
                    break;
                }
                // Both sides of '-' must be ClassSetCharacters (a ClassSetRange).
                if last_type != ClassSetOperandType::ClassSetCharacter {
                    return Err(self.report_error(Error::InvalidCharacterClass));
                }
                let from = character;
                let (_, ty, c) = self.parse_class_set_operand(&mut items, &mut strings)?;
                last_type = ty;
                character = c;
                if last_type != ClassSetOperandType::ClassSetCharacter {
                    return Err(self.report_error(Error::InvalidCharacterClass));
                }
                if from > character {
                    return Err(self.report_error(Error::OutOfOrderCharacterClass));
                }
                items.push(ClassItem::Range(from, character));
                last_type = ClassSetOperandType::ClassSetRange;
            } else {
                if last_type == ClassSetOperandType::ClassSetCharacter {
                    items.push(ClassItem::Range(character, character));
                }
                let (operand, ty, c) = self.parse_class_set_operand(&mut items, &mut strings)?;
                last_type = ty;
                character = c;
                may_contain |= may_contain_strings(last_type, &operand);
                // Add the operand started so far, then the tree.
                if let Some(t) = operand.tree {
                    if !items.is_empty() || !strings.is_empty() {
                        may_contain |= !strings.is_empty();
                        operands.push(Tree::ClassSetOperand(ClassSetOperand {
                            items: std::mem::take(&mut items),
                            strings: std::mem::take(&mut strings),
                        }));
                    }
                    operands.push(t);
                }
            }
        }
        if !self.has_more() {
            return Err(self.report_error(Error::UnterminatedCharacterClass));
        }
        if last_type == ClassSetOperandType::ClassSetCharacter {
            items.push(ClassItem::Range(character, character));
        }
        // Add the operand started so far.
        if !items.is_empty() || !strings.is_empty() {
            may_contain |= !strings.is_empty();
            operands.push(Tree::ClassSetOperand(ClassSetOperand { items, strings }));
        }
        debug_assert_eq!(self.current(), ch(b']'));
        self.advance();
        if is_negated && may_contain {
            return Err(self.report_error(Error::NegatedCharacterClassWithStrings));
        }
        let tree = if operands.is_empty() {
            // No operand (V8: e.g. [\P{Any}] gives an empty range list).
            ClassSetExpression::empty(is_negated)
        } else {
            Tree::ClassSetExpression(ClassSetExpression {
                operation: ClassSetOperation::Union,
                negated: is_negated,
                may_contain_strings: may_contain,
                operands,
            })
        };
        Ok(ClassSetResult::new(Some(tree), may_contain))
    }

    /// ClassIntersection (V8 `ParseClassIntersection`).
    fn parse_class_intersection(
        &mut self,
        is_negated: bool,
        first_operand: ClassSetResult,
        first_operand_type: ClassSetOperandType,
    ) -> PResult<ClassSetResult> {
        debug_assert!(self.current() == ch(b'&') && self.next() == ch(b'&'));
        let mut may_contain = may_contain_strings(first_operand_type, &first_operand);
        let mut operands = vec![first_operand.tree.expect("an operand tree")];
        while self.has_more() && self.current() != ch(b']') {
            if self.current() != ch(b'&') || self.next() != ch(b'&') {
                return Err(self.report_error(Error::InvalidClassSetOperation));
            }
            self.advance_by(2);
            // [lookahead ≠ &]
            if self.current() == ch(b'&') {
                return Err(self.report_error(Error::InvalidCharacterInClass));
            }
            let (operand, ty) = self.parse_class_set_operand_tree()?;
            may_contain &= may_contain_strings(ty, &operand);
            operands.push(operand.tree.expect("an operand tree"));
        }
        if !self.has_more() {
            return Err(self.report_error(Error::UnterminatedCharacterClass));
        }
        if is_negated && may_contain {
            return Err(self.report_error(Error::NegatedCharacterClassWithStrings));
        }
        debug_assert_eq!(self.current(), ch(b']'));
        self.advance();
        Ok(ClassSetResult::new(
            Some(Tree::ClassSetExpression(ClassSetExpression {
                operation: ClassSetOperation::Intersection,
                negated: is_negated,
                may_contain_strings: may_contain,
                operands,
            })),
            may_contain,
        ))
    }

    /// ClassSubtraction (V8 `ParseClassSubtraction`).
    fn parse_class_subtraction(
        &mut self,
        is_negated: bool,
        first_operand: ClassSetResult,
        first_operand_type: ClassSetOperandType,
    ) -> PResult<ClassSetResult> {
        debug_assert!(self.current() == ch(b'-') && self.next() == ch(b'-'));
        let may_contain = may_contain_strings(first_operand_type, &first_operand);
        if is_negated && may_contain {
            return Err(self.report_error(Error::NegatedCharacterClassWithStrings));
        }
        let mut operands = vec![first_operand.tree.expect("an operand tree")];
        while self.has_more() && self.current() != ch(b']') {
            if self.current() != ch(b'-') || self.next() != ch(b'-') {
                return Err(self.report_error(Error::InvalidClassSetOperation));
            }
            self.advance_by(2);
            let (operand, _) = self.parse_class_set_operand_tree()?;
            operands.push(operand.tree.expect("an operand tree"));
        }
        if !self.has_more() {
            return Err(self.report_error(Error::UnterminatedCharacterClass));
        }
        debug_assert_eq!(self.current(), ch(b']'));
        self.advance();
        Ok(ClassSetResult::new(
            Some(Tree::ClassSetExpression(ClassSetExpression {
                operation: ClassSetOperation::Subtraction,
                negated: is_negated,
                may_contain_strings: may_contain,
                operands,
            })),
            may_contain,
        ))
    }

    /// CharacterClass at `current` = `[` (V8 `ParseCharacterClass`). With `v` it nests, so
    /// the depth counts toward [`MAX_NESTING`].
    fn parse_character_class(&mut self) -> PResult<ClassSetResult> {
        if self.states.len() + self.class_depth >= MAX_NESTING {
            return Err(self.report_error(Error::StackOverflow));
        }
        self.class_depth += 1;
        let r = self.parse_character_class_body();
        self.class_depth -= 1;
        r
    }

    fn parse_character_class_body(&mut self) -> PResult<ClassSetResult> {
        debug_assert_eq!(self.current(), ch(b'['));
        self.advance();
        let mut is_negated = false;
        if self.current() == ch(b'^') {
            is_negated = true;
            self.advance();
        }
        let mut items = Vec::with_capacity(2);
        if self.current() == ch(b']') {
            self.advance();
            let tree = if self.unicode_sets() {
                ClassSetExpression::empty(is_negated)
            } else {
                Tree::ClassRanges(ClassRanges {
                    items,
                    negated: is_negated,
                })
            };
            return Ok(ClassSetResult::new(Some(tree), false));
        }
        if !self.unicode_sets() {
            // V8 adds the case equivalents of u+i classes here; in RX1 the compiler does.
            self.parse_class_ranges(&mut items)?;
            if !self.has_more() {
                return Err(self.report_error(Error::UnterminatedCharacterClass));
            }
            debug_assert_eq!(self.current(), ch(b']'));
            self.advance();
            return Ok(ClassSetResult::new(
                Some(Tree::ClassRanges(ClassRanges {
                    items,
                    negated: is_negated,
                })),
                false,
            ));
        }
        let mut strings = ClassStrings::new();
        let (mut operand, operand_type, character) =
            self.parse_class_set_operand(&mut items, &mut strings)?;
        let c = self.current();
        let next = self.next();
        if (c == ch(b'-') && next == ch(b'-')) || (c == ch(b'&') && next == ch(b'&')) {
            if operand.tree.is_none() {
                if operand_type == ClassSetOperandType::ClassSetCharacter {
                    items.push(ClassItem::Range(character, character));
                }
                operand.tree = Some(Tree::ClassSetOperand(ClassSetOperand {
                    items: std::mem::take(&mut items),
                    strings: std::mem::take(&mut strings),
                }));
            }
            return if c == ch(b'-') {
                self.parse_class_subtraction(is_negated, operand, operand_type)
            } else {
                self.parse_class_intersection(is_negated, operand, operand_type)
            };
        }
        // A ClassSetRange is handled in parse_class_union.
        self.parse_class_union(is_negated, operand, operand_type, items, strings, character)
    }

    // ---- entry -----------------------------------------------------------------------------

    /// V8 `Parse`.
    pub(crate) fn parse(mut self) -> Result<Pattern, ParseError> {
        match self.parse_pattern() {
            Ok(tree) => {
                debug_assert!(self.error.is_none());
                let capture_count = self.captures_started;
                // A literal atom as long as the input is the input itself.
                let simple = matches!(&tree, Tree::Atom(a) if a.len() == self.input.len())
                    && capture_count == 0;
                let named_captures = self
                    .named_captures
                    .take()
                    .map(|m| {
                        m.into_iter()
                            .flat_map(|(name, indices)| {
                                indices.into_iter().map(move |i| (name.clone(), i))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(Pattern {
                    tree,
                    capture_count,
                    named_captures,
                    simple,
                    contains_anchor: self.contains_anchor,
                })
            }
            Err(Failed) => Err(ParseError {
                error: self.error.expect("a failed parse has an error"),
                pos: self.error_pos,
            }),
        }
    }
}

fn push_code_unit(v: &mut Vec<u16>, c: u32) {
    if c <= 0xffff {
        v.push(c as u16);
    } else {
        v.push((0xd800 + ((c - 0x10000) >> 10)) as u16);
        v.push((0xdc00 + ((c - 0x10000) & 0x3ff)) as u16);
    }
}

/// Fills the group indices of every named back reference (the walk depth is at most
/// [`MAX_NESTING`] levels of groups).
fn fill_named_references(tree: &mut Tree, map: &BTreeMap<Vec<u16>, Vec<u32>>) {
    match tree {
        Tree::BackReference(r) => {
            if let Some(name) = &r.name {
                r.captures = map[name].clone();
            }
        }
        Tree::Disjunction(v) | Tree::Alternative(v) | Tree::Text(v) => {
            for t in v {
                fill_named_references(t, map);
            }
        }
        Tree::Quantifier(q) => fill_named_references(&mut q.body, map),
        Tree::Capture(c) => fill_named_references(&mut c.body, map),
        Tree::Group(g) => fill_named_references(&mut g.body, map),
        Tree::Lookaround(l) => fill_named_references(&mut l.body, map),
        Tree::Assertion(_)
        | Tree::ClassRanges(_)
        | Tree::ClassSetOperand(_)
        | Tree::ClassSetExpression(_)
        | Tree::Atom(_)
        | Tree::Empty => {}
    }
}
