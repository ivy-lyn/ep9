//! A test-only reference walker of the node graph: it runs a [`Program`] on a subject by
//! following the nodes depth first, with the meaning V8's code generator gives each node
//! (regexp-compiler.cc `*::Emit`, regexp-macro-assembler.cc, regexp-interpreter.cc). It has
//! no speed and no bytecode; it lets the unit tests check the graph against node (V8).
//!
//! ```text
//! run(node, position, registers) -> Fail | Accept(registers) | the end of a look-around body
//!   a choice tries each alternative whose guard holds, with a copy of the registers
//!   a look-around runs its body once (atomic), then the rest from the saved position
//!   undo on backtrack = the caller's copy of the registers (V8 pushes and clears them)
//! ```
//!
//! A step budget bounds the walk ([`Outcome::Budget`]); no wall time is read.

use crate::charset;
use crate::special_case::{self, Mode};

use super::Program;
use super::node::{
    Action, AssertionKind, ChoiceKind, End, NodeId, NodeKind, Relation, TextElementKind,
};
use super::quick_check::preload_characters;

/// The groups of a match (`None` for an unset group), `None` for no match.
pub(crate) type Groups = Option<Vec<Option<(usize, usize)>>>;

/// How a walk from a node ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Fail,
    Accept(Vec<i64>),
    /// The body of the positive look-around whose success node is `node` matched.
    PositiveSuccess {
        node: NodeId,
        registers: Vec<i64>,
    },
    /// The body of the negative look-around with this stack register matched.
    NegativeSuccess(u32),
    Budget,
}

pub(crate) struct Walker<'a> {
    program: &'a Program,
    subject: &'a [u16],
    steps: u64,
    limit: u64,
    /// The analysis facts that a successful path broke (eats-at-least, quick checks).
    pub(crate) problems: Vec<String>,
}

fn is_word(c: u16) -> bool {
    matches!(c, 0x30..=0x39 | 0x41..=0x5a | 0x5f | 0x61..=0x7a)
}

fn is_line_terminator(c: u16) -> bool {
    matches!(c, 0x0a | 0x0d | 0x2028 | 0x2029)
}

fn is_lead(c: u16) -> bool {
    (0xd800..=0xdbff).contains(&c)
}

fn is_trail(c: u16) -> bool {
    (0xdc00..=0xdfff).contains(&c)
}

impl<'a> Walker<'a> {
    pub(crate) fn new(program: &'a Program, subject: &'a [u16], limit: u64) -> Walker<'a> {
        if program.one_byte() {
            assert!(
                subject.iter().all(|&c| c <= 0xff),
                "a one-byte program needs Latin-1"
            );
        }
        Walker {
            program,
            subject,
            steps: 0,
            limit,
            problems: Vec::new(),
        }
    }

    /// The match from position 0 (V8 exec with lastIndex 0): the capture pairs, `None` for
    /// no match; `Err` when the budget ran out.
    pub(crate) fn exec(&mut self) -> Result<Groups, ()> {
        let registers = vec![-1; self.program.register_count() as usize];
        match self.run(self.program.start(), 0, registers) {
            Outcome::Accept(r) => {
                let n = self.program.capture_count() as usize + 1;
                Ok(Some(
                    (0..n)
                        .map(|i| {
                            let (s, e) = (r[2 * i], r[2 * i + 1]);
                            (s >= 0 && e >= 0).then_some((s as usize, e as usize))
                        })
                        .collect(),
                ))
            }
            Outcome::Fail => Ok(None),
            Outcome::Budget => Err(()),
            other => panic!("a walk ended at {other:?}"),
        }
    }

    fn at(&self, i: i64) -> Option<u16> {
        usize::try_from(i)
            .ok()
            .and_then(|i| self.subject.get(i))
            .copied()
    }

    fn char_matches(&self, pattern: u16, flags: crate::Flags, c: u16) -> bool {
        if pattern == c {
            return true;
        }
        flags.ignore_case()
            && special_case::case_independent_letters(
                pattern,
                flags.either_unicode(),
                self.program.one_byte(),
            )
            .contains(&c)
    }

    /// V8 `CaseInsensitiveCompare*` (two-byte) and the interpreter's Latin-1 form.
    fn back_reference_equal(&self, a: &[u16], b: &[u16], unicode: bool) -> bool {
        if self.program.one_byte() {
            return a.iter().zip(b).all(|(&x, &y)| {
                let (x, y) = (u32::from(x), u32::from(y));
                if x == y {
                    return true;
                }
                let (x, y) = (x | 0x20, y | 0x20);
                x == y && ((0x61..=0x7a).contains(&x) || ((0xe0..=0xfe).contains(&x) && x != 0xf7))
            });
        }
        if !unicode {
            return a.iter().zip(b).all(|(&x, &y)| {
                x == y
                    || special_case::equivalence_key(u32::from(x), Mode::NonUnicode)
                        == special_case::equivalence_key(u32::from(y), Mode::NonUnicode)
            });
        }
        let decode = |s: &[u16]| -> Vec<(usize, u32)> {
            let mut out = Vec::new();
            let mut i = 0;
            while i < s.len() {
                if is_lead(s[i]) && i + 1 < s.len() && is_trail(s[i + 1]) {
                    let c = 0x10000
                        + ((u32::from(s[i]) - 0xd800) << 10)
                        + (u32::from(s[i + 1]) - 0xdc00);
                    i += 2;
                    out.push((i, c));
                } else {
                    out.push((i + 1, u32::from(s[i])));
                    i += 1;
                }
            }
            out
        };
        let (da, db) = (decode(a), decode(b));
        for (k, &(ia, ca)) in da.iter().enumerate() {
            let Some(&(ib, cb)) = db.get(k) else {
                return false;
            };
            if ia != ib {
                return false;
            }
            if ca != cb
                && special_case::equivalence_key(ca, Mode::Unicode)
                    != special_case::equivalence_key(cb, Mode::Unicode)
            {
                return false;
            }
        }
        true
    }

    /// Runs from `id` and checks the analysis on a path that succeeds: the subject has the
    /// characters the node eats at least.
    pub(crate) fn run(&mut self, id: NodeId, pos: i64, regs: Vec<i64>) -> Outcome {
        self.steps += 1;
        if self.steps > self.limit {
            return Outcome::Budget;
        }
        let out = self.step(id, pos, regs);
        if !matches!(out, Outcome::Fail | Outcome::Budget) {
            let eats = i64::from(self.program.eats_at_least(id, pos > 0));
            if (self.subject.len() as i64) - pos < eats {
                self.problems.push(format!(
                    "node {} at {pos} succeeds with {} left, eats at least {eats}",
                    id.index(),
                    self.subject.len() as i64 - pos
                ));
            }
        }
        out
    }

    /// A choice alternative `alt` of `choice` succeeded at `pos`: the subject passes its
    /// quick check (V8 loads the characters the choice eats at least).
    fn check_quick(&mut self, choice: NodeId, alt: NodeId, pos: i64) {
        let program = self.program;
        let n = preload_characters(program.eats_at_least(choice, pos > 0), program.one_byte());
        if n == 0 || pos + n as i64 > self.subject.len() as i64 {
            return;
        }
        let Some(details) = program.quick_check(alt, n, pos > 0) else {
            return;
        };
        let shift = if program.one_byte() { 8 } else { 16 };
        let mut word = 0u32;
        for i in 0..n {
            word |= u32::from(self.subject[pos as usize + i]) << (i * shift);
        }
        if word & details.mask() != details.value() {
            self.problems.push(format!(
                "choice {} alternative {} at {pos}: quick check {:#x}/{:#x} rejects {word:#x}",
                choice.index(),
                alt.index(),
                details.mask(),
                details.value()
            ));
        }
    }

    fn step(&mut self, id: NodeId, pos: i64, mut regs: Vec<i64>) -> Outcome {
        let program = self.program;
        let node = program.node(id);
        let flags = node.flags;
        match &node.kind {
            NodeKind::End(End::Accept) => Outcome::Accept(regs),
            NodeKind::End(End::Backtrack) => Outcome::Fail,
            NodeKind::End(End::NegativeSubmatchSuccess {
                stack_pointer_register,
                ..
            }) => Outcome::NegativeSuccess(*stack_pointer_register),
            NodeKind::Action(a) => {
                let next = a.on_success;
                match a.action {
                    Action::SetRegisterForLoop { register, value } => {
                        regs[register as usize] = i64::from(value);
                    }
                    Action::IncrementRegister { register } => regs[register as usize] += 1,
                    // V8's RESTORE_POSITION stores the position too; only its undo differs.
                    Action::StorePosition { register, .. }
                    | Action::RestorePosition { register } => {
                        regs[register as usize] = pos;
                    }
                    Action::ClearCaptures { from, to } => {
                        for r in from..=to {
                            regs[r as usize] = -1;
                        }
                    }
                    Action::EatsAtLeast { .. } => {}
                    Action::EmptyMatchCheck {
                        start_register,
                        repetition_register,
                        repetition_limit,
                    } => {
                        let below_min = repetition_register
                            .is_some_and(|r| regs[r as usize] < i64::from(repetition_limit));
                        if !below_min && regs[start_register as usize] == pos {
                            return Outcome::Fail;
                        }
                    }
                    Action::BeginPositiveSubmatch { success_node, .. } => {
                        return match self.run(next, pos, regs) {
                            Outcome::PositiveSuccess { node, registers }
                                if node == success_node =>
                            {
                                let after = program
                                    .node(success_node)
                                    .on_success()
                                    .expect("a successor");
                                self.run(after, pos, registers)
                            }
                            other => other,
                        };
                    }
                    Action::PositiveSubmatchSuccess { .. } => {
                        return Outcome::PositiveSuccess {
                            node: id,
                            registers: regs,
                        };
                    }
                    Action::BeginNegativeSubmatch {
                        stack_pointer_register,
                        ..
                    } => {
                        let NodeKind::Choice(c) = &program.node(next).kind else {
                            panic!("a negative look-around choice");
                        };
                        let (body, rest) = (c.alternatives[0].node, c.alternatives[1].node);
                        return match self.run(body, pos, regs.clone()) {
                            Outcome::NegativeSuccess(s) if s == stack_pointer_register => {
                                Outcome::Fail
                            }
                            Outcome::Fail => self.run(rest, pos, regs),
                            other => other,
                        };
                    }
                }
                self.run(next, pos, regs)
            }
            NodeKind::Text(t) => {
                let length = i64::from(t.length());
                let base = if t.read_backward { pos - length } else { pos };
                if base < 0 || base + length > self.subject.len() as i64 {
                    return Outcome::Fail;
                }
                for e in &t.elements {
                    let at = base + i64::from(e.cp_offset);
                    match &e.kind {
                        TextElementKind::Atom(units) => {
                            for (k, &u) in units.iter().enumerate() {
                                let c = self.at(at + k as i64).expect("in bounds");
                                if !self.char_matches(u, flags, c) {
                                    return Outcome::Fail;
                                }
                            }
                        }
                        TextElementKind::Class(cr) => {
                            let c = self.at(at).expect("in bounds");
                            let mut ranges = cr.ranges.clone();
                            charset::canonicalize(&mut ranges);
                            if charset::contains(&ranges, u32::from(c)) == cr.negated {
                                return Outcome::Fail;
                            }
                        }
                    }
                }
                let after = if t.read_backward { base } else { base + length };
                self.run(t.on_success, after, regs)
            }
            NodeKind::Assertion(a) => {
                let ok = match a.kind {
                    AssertionKind::AtStart => pos == 0,
                    AssertionKind::AtEnd => pos == self.subject.len() as i64,
                    AssertionKind::AfterNewline => {
                        pos == 0 || self.at(pos - 1).is_some_and(is_line_terminator)
                    }
                    AssertionKind::AtBoundary | AssertionKind::AtNonBoundary => {
                        let before = self.at(pos - 1).is_some_and(is_word);
                        let after = self.at(pos).is_some_and(is_word);
                        (before != after) == (a.kind == AssertionKind::AtBoundary)
                    }
                };
                if ok {
                    self.run(a.on_success, pos, regs)
                } else {
                    Outcome::Fail
                }
            }
            NodeKind::BackReference(b) => {
                let from = regs[b.start_register as usize];
                let len = regs[b.end_register as usize] - from;
                let mut pos = pos;
                if from >= 0 && len > 0 {
                    let (start, end) = if b.read_backward {
                        (pos - len, pos)
                    } else {
                        (pos, pos + len)
                    };
                    if start < 0 || end > self.subject.len() as i64 {
                        return Outcome::Fail;
                    }
                    let captured = &self.subject[from as usize..(from + len) as usize];
                    let here = &self.subject[start as usize..end as usize];
                    let equal = if flags.ignore_case() {
                        self.back_reference_equal(captured, here, flags.either_unicode())
                    } else {
                        captured == here
                    };
                    if !equal {
                        return Outcome::Fail;
                    }
                    pos = if b.read_backward { start } else { end };
                }
                // The reference may not end inside a surrogate pair.
                if flags.either_unicode()
                    && !program.one_byte()
                    && self.at(pos).is_some_and(is_trail)
                    && self.at(pos - 1).is_some_and(is_lead)
                {
                    return Outcome::Fail;
                }
                self.run(b.on_success, pos, regs)
            }
            NodeKind::UnanchoredAdvance { on_success } => {
                let Some(c) = self.at(pos) else {
                    return Outcome::Fail;
                };
                let step = if flags.either_unicode()
                    && !program.one_byte()
                    && is_lead(c)
                    && self.at(pos + 1).is_some_and(is_trail)
                {
                    2
                } else {
                    1
                };
                self.run(*on_success, pos + step, regs)
            }
            NodeKind::Choice(c) => {
                debug_assert!(c.kind != ChoiceKind::NegativeLookaround);
                for alt in &c.alternatives {
                    if let Some(g) = alt.guard {
                        let v = regs[g.register as usize];
                        let holds = match g.relation {
                            Relation::Lt => v < i64::from(g.value),
                            Relation::Geq => v >= i64::from(g.value),
                        };
                        if !holds {
                            continue;
                        }
                    }
                    match self.run(alt.node, pos, regs.clone()) {
                        Outcome::Fail => {}
                        Outcome::Budget => return Outcome::Budget,
                        other => {
                            if !c.read_backward() {
                                self.check_quick(id, alt.node, pos);
                            }
                            return other;
                        }
                    }
                }
                Outcome::Fail
            }
        }
    }
}
