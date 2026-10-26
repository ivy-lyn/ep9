#![allow(dead_code)] // m4-stub
//! RegExp backends (docs/plan/08-repo.md section 2; docs/plan/04-execution.md "RX0").
//!
//! ```text
//!   ep9_builtins --Cx part-2 methods--> ep9_vm --Backend::compile / exec--> ep9_regexp
//!                                         |                                   |
//!                     Runtime.regexp_programs (one Program per RegExp)     RX0: regress 0.12.0
//! ```
//!
//! Only `ep9_vm` depends on this crate (.github/scripts/dep-guards.sh check 14). Builtins reach
//! it through `Cx` methods, never directly. A [`Backend::Program`] is plain Rust memory with no
//! GC value in it, so the VM keeps it in a per-isolate table and a RegExp cell holds its index.
//!
//! | Backend | State |
//! |---|---|
//! | RX0 (regress 0.12.0, then its fork) | Not built: this file has the trait signatures only (M4-P0 stub; the RX0A role fills it) |
//! | RX1, RX2 (irregexp port) | Not started (M2-M5) |
#![forbid(unsafe_code)]

/// The flags of a RegExp (`dgimsuvy`), one bit each. The backend reads `i`, `m`, `s`, `u` and
/// `v`; `d`, `g` and `y` are for the caller (the `lastIndex` loop and the match indices).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Flags(u8);

impl Flags {
    /// `d`: the result has match indices.
    pub const HAS_INDICES: Flags = Flags(1 << 0);
    /// `g`: global.
    pub const GLOBAL: Flags = Flags(1 << 1);
    /// `i`: ignore case.
    pub const IGNORE_CASE: Flags = Flags(1 << 2);
    /// `m`: `^` and `$` also match at line ends.
    pub const MULTILINE: Flags = Flags(1 << 3);
    /// `s`: `.` also matches line ends.
    pub const DOT_ALL: Flags = Flags(1 << 4);
    /// `u`: Unicode mode.
    pub const UNICODE: Flags = Flags(1 << 5);
    /// `v`: Unicode sets mode.
    pub const UNICODE_SETS: Flags = Flags(1 << 6);
    /// `y`: sticky (match only at the start index).
    pub const STICKY: Flags = Flags(1 << 7);

    /// No flag.
    pub const fn empty() -> Flags {
        Flags(0)
    }

    /// The flags of `bits` (bit order as the constants above).
    pub const fn from_bits(bits: u8) -> Flags {
        Flags(bits)
    }

    /// The bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// True if every flag of `other` is set.
    pub const fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    /// Both sets of flags.
    pub const fn union(self, other: Flags) -> Flags {
        Flags(self.0 | other.0)
    }
}

/// The subject string, in one of the two VM string forms (H19). Indices are code units.
#[derive(Clone, Copy, Debug)]
pub enum Input<'a> {
    /// One byte per code unit (Latin-1).
    Latin1(&'a [u8]),
    /// UTF-16 code units, lone surrogates allowed.
    Utf16(&'a [u16]),
}

impl Input<'_> {
    /// The length in code units.
    pub fn len(&self) -> usize {
        match self {
            Input::Latin1(s) => s.len(),
            Input::Utf16(s) => s.len(),
        }
    }

    /// True for the empty string.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A pattern that does not compile. The VM throws a SyntaxError with this message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxErr {
    /// The message, without the "SyntaxError: " prefix.
    pub message: String,
}

/// How one [`Backend::exec`] call ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecResult {
    /// A match. `caps[0]` holds the whole match; `caps[n]` group n, or `None`.
    Match,
    /// No match from `start` on (sticky: no match at `start`).
    NoMatch,
    /// `poll` returned true: the VM has an interrupt (termination, H23). `caps` is unspecified.
    Interrupted,
    /// The backtrack budget ran out. The VM decides what the script sees. `caps` is unspecified.
    BudgetExceeded,
}

/// A RegExp engine. Every function is static: a backend keeps its state in [`Backend::Program`]
/// and in the arguments.
pub trait Backend {
    /// A compiled pattern: Rust memory only, no GC value (the VM keeps it outside the heap).
    type Program;

    /// Compiles `pattern` (the RegExp source as UTF-16 code units) with `flags`.
    fn compile(pattern: &[u16], flags: Flags) -> Result<Self::Program, SyntaxErr>;

    /// Runs `program` on `input` from code unit `start`. On [`ExecResult::Match`] the capture
    /// ranges (start, end) in code units are in `caps`; `caps.len()` is the group count + 1.
    /// The backend calls `poll` from time to time during a long match (between match attempts
    /// at least); a `true` return stops it with [`ExecResult::Interrupted`].
    fn exec(
        program: &Self::Program,
        input: Input<'_>,
        start: usize,
        caps: &mut [Option<(usize, usize)>],
        poll: &mut dyn FnMut() -> bool,
    ) -> ExecResult;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test backend: the pattern is a literal, matched code unit by code unit.
    struct Literal;

    impl Backend for Literal {
        type Program = Vec<u16>;

        fn compile(pattern: &[u16], flags: Flags) -> Result<Vec<u16>, SyntaxErr> {
            if flags.contains(Flags::UNICODE_SETS) && flags.contains(Flags::UNICODE) {
                return Err(SyntaxErr {
                    message: "u and v together".into(),
                });
            }
            Ok(pattern.to_vec())
        }

        fn exec(
            program: &Vec<u16>,
            input: Input<'_>,
            start: usize,
            caps: &mut [Option<(usize, usize)>],
            poll: &mut dyn FnMut() -> bool,
        ) -> ExecResult {
            let unit = |i: usize| match input {
                Input::Latin1(s) => u16::from(s[i]),
                Input::Utf16(s) => s[i],
            };
            let mut at = start;
            while at + program.len() <= input.len() {
                if poll() {
                    return ExecResult::Interrupted;
                }
                if program.iter().enumerate().all(|(k, &c)| unit(at + k) == c) {
                    caps[0] = Some((at, at + program.len()));
                    return ExecResult::Match;
                }
                at += 1;
            }
            ExecResult::NoMatch
        }
    }

    fn utf16(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn trait_signatures_work_for_both_input_forms() {
        let p = Literal::compile(&utf16("lo"), Flags::GLOBAL).unwrap();
        let mut caps = [None];
        let mut never = || false;
        let r = Literal::exec(&p, Input::Latin1(b"hello"), 0, &mut caps, &mut never);
        assert_eq!((r, caps[0]), (ExecResult::Match, Some((3, 5))));
        let text = utf16("h\u{e9}llo");
        let r = Literal::exec(&p, Input::Utf16(&text), 4, &mut caps, &mut never);
        assert_eq!(r, ExecResult::NoMatch);
        let mut polls = 0;
        let mut stop = || {
            polls += 1;
            true
        };
        let r = Literal::exec(&p, Input::Utf16(&text), 0, &mut caps, &mut stop);
        assert_eq!((r, polls), (ExecResult::Interrupted, 1));
        let both = Flags::UNICODE.union(Flags::UNICODE_SETS);
        assert!(Literal::compile(&[], both).is_err());
    }

    #[test]
    fn flags_bits() {
        assert_eq!(Flags::empty().bits(), 0);
        assert_eq!(Flags::from_bits(0xff).bits(), 0xff);
        let f = Flags::IGNORE_CASE.union(Flags::STICKY);
        assert!(f.contains(Flags::STICKY) && !f.contains(Flags::GLOBAL));
        assert!(Input::Latin1(b"").is_empty());
    }
}
