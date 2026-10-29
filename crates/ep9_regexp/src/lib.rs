//! RegExp backends (docs/plan/08-repo.md section 2; docs/plan/04-execution.md "RX0").
//!
//! ```text
//!   ep9_builtins --Cx part-2 methods--> ep9_vm --Backend::compile / exec--> ep9_regexp
//!                                         |                                   |
//!                     Runtime.regexp_programs (one Program per RegExp)     RX0: regress fork
//! ```
//!
//! Only `ep9_vm` depends on this crate (.github/scripts/dep-guards.sh check 14). Builtins reach
//! it through `Cx` methods, never directly. A [`Backend::Program`] is plain Rust memory with no
//! GC value in it, so the VM keeps it in a per-isolate table and a RegExp cell holds its index.
//!
//! | Backend | State |
//! |---|---|
//! | RX0 ([`Rx0`]: regress 0.12.0 from the fork checkout `third_party/regress` through `[patch.crates-io]`: upstream v0.12.0 at `third_party/regress.commit`, plus `third_party/patches/regress/`; V9-D15) | Built (M4-P2 RX0A). Fork wired in M5-P0; the 7 fork patches in M5-P3 (RX0b) |
//! | RX1, RX2 (irregexp port) | Not started (M2-M5) |
//!
//! The fork patches (M5-P3 RX0b, `third_party/patches/regress/`, each an upstreamable commit
//! with its own test in regress `tests/`) and what [`Rx0`] uses of them:
//!
//! | Patch | regress API | Here |
//! |---|---|---|
//! | 0001 poll + budget | `Limits`, `Stopped` | `poll` before the search and every [`POLL_INTERVAL`] steps inside it (a start position, a backtrack, a loop iteration, a chunk of the start-position scan): an interrupt ends an exec within one interval. No budget: JavaScript has no backtrack limit, so [`ExecResult::BudgetExceeded`] is never returned (one rule for every execution mode, MC-w20-16) |
//! | 0002 single-position exec | `match_at_limited`, `exec_into(.., anchored)` | sticky (`y`) is one attempt at `start`, not a search (plan v9 R16) |
//! | 0003 duplicate-name backref | `BackRefAny` | `\k<x>` of a name that several groups share matches the group that took part (test262 duplicate-names-*) |
//! | 0004 non-Unicode `i` ASCII rule | `canonicalize_legacy` | Canonicalize (§22.2.2.7.3) without `u`/`v`: `ſ`, `ı`, the Kelvin sign do not match ASCII letters; the node golden has no KNOWN line |
//! | 0005 Latin-1 input + start predicates | `Text::Latin1`, `unit_pred` | a Latin-1 subject is read in place (no UTF-16 copy); literal prefixes, first-unit sets and `^` skip positions |
//! | 0006 caller-owned scratch | `Scratch`, `exec_into` | [`Backend::Scratch`]: the engine's stacks and the capture ranges are reused across execs (no allocation per exec once grown) |
//! | 0007 flat string-property tables | `StringSetTable` (internal) | nothing to call: the tables of `\p{RGI_Emoji}` and the other properties of strings hold no pointer per string, so the binary has 7,905 fewer load-time fixups (W-REGEXP-1) |
//!
//! RX0 fixes in this crate, before regress sees the pattern or the start: `legacy_chars`
//! (`\u{` and surrogate escapes without `u`; a group name as code points), `unicode_chars` (a
//! lead-surrogate escape before another `\u` escape, `u` and `v`) and the start inside a
//! surrogate pair (Unicode mode starts at the pair, as V8). The node golden
//! `tests/golden/node-pairs.txt` (535 cases, node v24.22.1) checks the backend.
#![forbid(unsafe_code)]

/// The flags of a RegExp (`dgimsuvy`), one bit each. The backend reads `i`, `m`, `s`, `u`,
/// `v` and `y`; `d` and `g` are for the caller (the match indices and the `lastIndex` loop).
/// The bit order is the bytecode's (`CreateRegExpLiteral` flags operand, d = bit 0 ... y =
/// bit 7; `ep9_frontend` bytecodegen/literal.rs `regexp_flags`).
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

    /// The flag letters in bit order (also the order of the `flags` getter, §22.2.6.4).
    const LETTERS: [u8; 8] = *b"dgimsuvy";

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

    /// The flags of a flags string (RegExpInitialize, §22.2.3.3 step 5): `None` if a code
    /// unit is not one of `dgimsuvy` or a flag comes twice. `u` with `v` is a compile error
    /// ([`Backend::compile`]), not a parse error here, as in the spec text.
    pub fn parse(units: &[u16]) -> Option<Flags> {
        let mut bits = 0u8;
        for &c in units {
            let at = Self::LETTERS.iter().position(|&l| u16::from(l) == c)?;
            let bit = 1u8 << at;
            if bits & bit != 0 {
                return None;
            }
            bits |= bit;
        }
        Some(Flags(bits))
    }

    /// The flags string in the canonical order `dgimsuvy` (what the `flags` getter of a
    /// RegExp with these flags returns).
    pub fn to_letters(self) -> String {
        let mut s = String::with_capacity(8);
        for (i, &l) in Self::LETTERS.iter().enumerate() {
            if self.0 & (1 << i) != 0 {
                s.push(char::from(l));
            }
        }
        s
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

/// A capture span: (start, end) in code units.
pub type Span = (usize, usize);

/// A RegExp engine. Every function is static: a backend keeps its state in [`Backend::Program`]
/// and in the arguments.
pub trait Backend {
    /// A compiled pattern: Rust memory only, no GC value (the VM keeps it outside the heap).
    type Program;

    /// Memory an exec reuses (the engine's stacks): Rust memory only, no GC value and no
    /// state of a finished exec. The VM keeps one per isolate.
    type Scratch: Default;

    /// Compiles `pattern` (the RegExp source as UTF-16 code units) with `flags`.
    fn compile(pattern: &[u16], flags: Flags) -> Result<Self::Program, SyntaxErr>;

    /// Runs `program` on `input` from code unit `start` (`start` > the length: no match). On
    /// [`ExecResult::Match`] the capture spans in code units are in `caps` (cleared first):
    /// `caps.len()` is the group count + 1, `caps[0]` is the whole match. The backend calls
    /// `poll` from time to time (at least once per call, before the search); a `true` return
    /// stops it with [`ExecResult::Interrupted`]. `scratch` is memory the backend may keep
    /// between calls.
    fn exec(
        program: &Self::Program,
        input: Input<'_>,
        start: usize,
        caps: &mut Vec<Option<Span>>,
        scratch: &mut Self::Scratch,
        poll: &mut dyn FnMut() -> bool,
    ) -> ExecResult;
}

// ---- RX0: regress 0.12.0 --------------------------------------------------------------------

/// The RX0 backend: regress 0.12.0 with the ep9 fork patches (backtracking; Latin-1 and
/// UTF-16 input).
///
/// ```text
/// compile  pattern units -> code points (u/v: surrogate pairs joined; else one per unit)
///          -> regress::Regex::from_unicode (flags i m s u v; v also sets regress `unicode`)
/// exec     poll; Latin-1 -> Text::Latin1 (in place); u/v -> Text::Utf16, else Text::Ucs2;
///          regress exec_into (sticky: one attempt at `start`), scratch reused, poll every
///          POLL_INTERVAL steps
/// ```
#[derive(Debug)]
pub enum Rx0 {}

/// A program of [`Rx0`].
#[derive(Debug)]
pub struct Rx0Program {
    re: regress::Regex,
    /// `u` or `v`: the input is code points (surrogate pairs are one character).
    unicode: bool,
    sticky: bool,
}

impl Rx0Program {
    /// True if the pattern runs in Unicode mode (`u` or `v`).
    pub fn is_unicode(&self) -> bool {
        self.unicode
    }
}

/// The steps of a search (a start position, a backtrack or a loop iteration) between two
/// calls of the exec's `poll`: an interrupt ends a running exec within this many steps.
pub const POLL_INTERVAL: u32 = 1024;

/// The scratch memory that [`Rx0`] keeps above this many bytes is dropped after the exec, so
/// one exec that backtracked a lot does not hold its stack for the life of the isolate.
const SCRATCH_KEEP_BYTES: usize = 1 << 20;

/// The memory of [`Rx0`] execs ([`Backend::Scratch`]): regress's stacks and the capture
/// ranges of the last match. Rust memory, no GC value.
#[derive(Debug, Default)]
pub struct Rx0Scratch {
    engine: regress::Scratch,
    ranges: Vec<Option<regress::Range>>,
}

/// The pattern as code points for regress: in Unicode mode a surrogate pair is one code point
/// (§22.2.2: the pattern is read as UTF-16 code points) and `unicode_chars` rewrites one
/// escape form; otherwise each code unit is one character, so `/\u{1F600}+/` without `u`
/// repeats the low surrogate only, as in V8, and `legacy_chars` rewrites two escape forms
/// that regress reads in the Unicode way.
fn pattern_chars(pattern: &[u16], unicode: bool) -> Vec<u32> {
    if !unicode {
        return legacy_chars(pattern);
    }
    unicode_chars(pattern)
}

/// The Unicode-mode (`u`, `v`) pattern for regress: surrogate pairs joined, and one rewrite
/// for a regress 0.12.0 escape-parser defect:
///
/// | Source | Spec | regress | Rewrite |
/// |---|---|---|---|
/// | `\uD800\u0061` (a lead-surrogate escape, then a `\u` that is not a trail-surrogate escape) | the lone code point U+D800, then the next escape | after the failed pair it rewinds to after the second `\u`, so `0061` is read as text | the lead escape becomes `\u{D800}` (the same code point; regress reads it without a look-ahead) |
///
/// A lead escape followed by a trail escape (a pair) or by no `\u` is left as is.
fn unicode_chars(pattern: &[u16]) -> Vec<u32> {
    const BACKSLASH: u32 = b'\\' as u32;
    const U: u32 = b'u' as u32;
    let cps: Vec<u32> = char::decode_utf16(pattern.iter().copied())
        .map(|r| match r {
            Ok(c) => u32::from(c),
            Err(e) => u32::from(e.unpaired_surrogate()),
        })
        .collect();
    let mut out = Vec::with_capacity(cps.len());
    let mut i = 0;
    while i < cps.len() {
        if cps[i] != BACKSLASH {
            out.push(cps[i]);
            i += 1;
            continue;
        }
        let lone_lead = cps.get(i + 1) == Some(&U)
            && hex4(&cps, i + 2).is_some_and(is_lead)
            && cps.get(i + 6) == Some(&BACKSLASH)
            && cps.get(i + 7) == Some(&U)
            && !hex4(&cps, i + 8).is_some_and(is_trail);
        if lone_lead {
            out.extend([BACKSLASH, U, u32::from(b'{')]);
            out.extend_from_slice(&cps[i + 2..i + 6]);
            out.push(u32::from(b'}'));
            i += 6;
        } else {
            // An escape: the backslash and the character after it (so `\\u` is not read
            // as an escape).
            let end = (i + 2).min(cps.len());
            out.extend_from_slice(&cps[i..end]);
            i = end;
        }
    }
    out
}

fn is_lead(u: u16) -> bool {
    (0xd800..=0xdbff).contains(&u)
}

fn is_trail(u: u16) -> bool {
    (0xdc00..=0xdfff).contains(&u)
}

/// The value of four hex digits at `at` (code units or code points), if there are four.
fn hex4<T: Copy + Into<u32>>(pattern: &[T], at: usize) -> Option<u16> {
    let digits = pattern.get(at..at + 4)?;
    let mut v = 0u16;
    for &d in digits {
        let d = char::from_u32(d.into())?.to_digit(16)?;
        v = v * 16 + d as u16;
    }
    Some(v)
}

/// The non-Unicode pattern for regress, one character per code unit, with two rewrites where
/// regress 0.12.0 reads the pattern as in Unicode mode (B.1.2, `[~UnicodeMode]`):
///
/// | Source | Spec (no `u`, no `v`) | regress | Rewrite |
/// |---|---|---|---|
/// | `\u{61}` | identity escape `u`, then the quantifier `{61}` | the code point U+0061 | `u{61}` |
/// | `\uD83D\uDE00` | two characters (two code units) | one code point U+1F600, which a UCS-2 input never holds | each surrogate escape becomes its raw code unit |
///
/// A group name (`(?<name>`, and `\k<name>` when the pattern has a group name) is copied as
/// code points: there a `\u` pair and a raw surrogate pair are one code point in both modes
/// (RegExpIdentifierName is `[+UnicodeMode]`); an unpaired surrogate stays one character.
/// Without any group name, `\k` is the B.1.2 identity escape `k` and the text after it is
/// read as code units like the rest of the pattern.
fn legacy_chars(pattern: &[u16]) -> Vec<u32> {
    const BACKSLASH: u16 = b'\\' as u16;
    let unit = |c: u8| u16::from(c);
    let named = has_group_names(pattern);
    let mut out: Vec<u32> = Vec::with_capacity(pattern.len());
    let mut in_class = false;
    let mut i = 0;
    // Copies from `from` through the next `>` (or to the end) as code points; returns the
    // next index.
    let copy_name = |out: &mut Vec<u32>, from: usize| -> usize {
        let end = pattern[from..]
            .iter()
            .position(|&c| c == unit(b'>'))
            .map_or(pattern.len(), |k| from + k + 1);
        out.extend(
            char::decode_utf16(pattern[from..end].iter().copied()).map(|r| match r {
                Ok(c) => u32::from(c),
                Err(e) => u32::from(e.unpaired_surrogate()),
            }),
        );
        end
    };
    while i < pattern.len() {
        let c = pattern[i];
        if c == BACKSLASH {
            let Some(&d) = pattern.get(i + 1) else {
                out.push(u32::from(c));
                break;
            };
            if d == unit(b'u') {
                if let Some(v) = hex4(pattern, i + 2) {
                    if is_lead(v) || is_trail(v) {
                        out.push(u32::from(v));
                    } else {
                        out.extend(pattern[i..i + 6].iter().map(|&c| u32::from(c)));
                    }
                    i += 6;
                    continue;
                }
                if pattern.get(i + 2) == Some(&unit(b'{')) {
                    out.push(u32::from(d));
                    i += 2;
                    continue;
                }
            }
            if d == unit(b'k') && named && !in_class && pattern.get(i + 2) == Some(&unit(b'<')) {
                i = copy_name(&mut out, i);
                continue;
            }
            out.extend([u32::from(c), u32::from(d)]);
            i += 2;
            continue;
        }
        if in_class {
            in_class = c != unit(b']');
        } else if c == unit(b'[') {
            in_class = true;
        } else if is_group_name_open(pattern, i) {
            i = copy_name(&mut out, i);
            continue;
        }
        out.push(u32::from(c));
        i += 1;
    }
    out
}

/// True if a `(?<` that opens a group name (not a `(?<=` or `(?<!` look-behind) starts at `i`.
fn is_group_name_open(pattern: &[u16], i: usize) -> bool {
    let unit = |c: u8| u16::from(c);
    pattern.get(i) == Some(&unit(b'('))
        && pattern.get(i + 1) == Some(&unit(b'?'))
        && pattern.get(i + 2) == Some(&unit(b'<'))
        && !matches!(pattern.get(i + 3), Some(&n) if n == unit(b'=') || n == unit(b'!'))
}

/// True if the pattern has a group name outside escapes and classes (B.1.2: then `\k<name>`
/// is a backreference, else `\k` is an identity escape). The same walk as `legacy_chars`.
fn has_group_names(pattern: &[u16]) -> bool {
    let unit = |c: u8| u16::from(c);
    let mut in_class = false;
    let mut i = 0;
    while i < pattern.len() {
        let c = pattern[i];
        if c == unit(b'\\') {
            i += 2;
            continue;
        }
        if in_class {
            in_class = c != unit(b']');
        } else if c == unit(b'[') {
            in_class = true;
        } else if is_group_name_open(pattern, i) {
            return true;
        }
        i += 1;
    }
    false
}

/// V8's message form: `Invalid regular expression: /<source>/<flags>: <reason>`.
fn syntax_error(pattern: &[u16], flags: Flags, reason: &str) -> SyntaxErr {
    SyntaxErr {
        message: format!(
            "Invalid regular expression: /{}/{}: {reason}",
            String::from_utf16_lossy(pattern),
            flags.to_letters()
        ),
    }
}

impl Backend for Rx0 {
    type Program = Rx0Program;
    type Scratch = Rx0Scratch;

    fn compile(pattern: &[u16], flags: Flags) -> Result<Rx0Program, SyntaxErr> {
        if flags.contains(Flags::UNICODE.union(Flags::UNICODE_SETS)) {
            // RegExpInitialize step 11 (§22.2.3.3): u and v together.
            return Err(SyntaxErr {
                message: format!(
                    "Invalid flags supplied to RegExp constructor '{}'",
                    flags.to_letters()
                ),
            });
        }
        let unicode = flags.contains(Flags::UNICODE) || flags.contains(Flags::UNICODE_SETS);
        let rf = regress::Flags {
            icase: flags.contains(Flags::IGNORE_CASE),
            multiline: flags.contains(Flags::MULTILINE),
            dot_all: flags.contains(Flags::DOT_ALL),
            no_opt: false,
            unicode,
            unicode_sets: flags.contains(Flags::UNICODE_SETS),
        };
        let chars = pattern_chars(pattern, unicode);
        let re = regress::Regex::from_unicode(chars.into_iter(), rf)
            .map_err(|e| syntax_error(pattern, flags, &e.text))?;
        Ok(Rx0Program {
            re,
            unicode,
            sticky: flags.contains(Flags::STICKY),
        })
    }

    fn exec(
        program: &Rx0Program,
        input: Input<'_>,
        start: usize,
        caps: &mut Vec<Option<Span>>,
        scratch: &mut Rx0Scratch,
        poll: &mut dyn FnMut() -> bool,
    ) -> ExecResult {
        if poll() {
            return ExecResult::Interrupted;
        }
        if start > input.len() {
            return ExecResult::NoMatch;
        }
        let mut start = start;
        let text = match input {
            // Latin-1 has no surrogates: the same text in either mode.
            Input::Latin1(b) => regress::Text::Latin1(b),
            Input::Utf16(units) if program.unicode => {
                // Unicode mode: a start inside a surrogate pair starts at the pair (V8;
                // RegExpBuiltinExec reads "the character obtained from element lastIndex",
                // §22.2.7.2 step 13).
                if start > 0
                    && start < units.len()
                    && is_trail(units[start])
                    && is_lead(units[start - 1])
                {
                    start -= 1;
                }
                regress::Text::Utf16(units)
            }
            Input::Utf16(units) => regress::Text::Ucs2(units),
        };
        let mut limits = regress::Limits {
            budget: None,
            poll_interval: POLL_INTERVAL,
            poll,
        };
        let Rx0Scratch { engine, ranges } = scratch;
        let r = program
            .re
            .exec_into(text, start, program.sticky, engine, ranges, &mut limits);
        let out = match r {
            Ok(true) => {
                caps.clear();
                caps.extend(ranges.iter().map(|r| r.as_ref().map(|r| (r.start, r.end))));
                ExecResult::Match
            }
            Ok(false) => ExecResult::NoMatch,
            Err(regress::Stopped::Interrupted) => ExecResult::Interrupted,
            Err(regress::Stopped::BudgetExceeded) => ExecResult::BudgetExceeded,
        };
        if engine.memory() > SCRATCH_KEEP_BYTES {
            *engine = regress::Scratch::default();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test backend: the pattern is a literal, matched code unit by code unit.
    struct Literal;

    impl Backend for Literal {
        type Program = Vec<u16>;
        type Scratch = ();

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
            caps: &mut Vec<Option<Span>>,
            _scratch: &mut (),
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
                    caps.clear();
                    caps.push(Some((at, at + program.len())));
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

    fn never() -> bool {
        false
    }

    #[test]
    fn trait_signatures_work_for_both_input_forms() {
        let p = Literal::compile(&utf16("lo"), Flags::GLOBAL).unwrap();
        let mut caps = Vec::new();
        let r = Literal::exec(
            &p,
            Input::Latin1(b"hello"),
            0,
            &mut caps,
            &mut (),
            &mut never,
        );
        assert_eq!((r, caps[0]), (ExecResult::Match, Some((3, 5))));
        let text = utf16("h\u{e9}llo");
        let r = Literal::exec(&p, Input::Utf16(&text), 4, &mut caps, &mut (), &mut never);
        assert_eq!(r, ExecResult::NoMatch);
        let mut polls = 0;
        let mut stop = || {
            polls += 1;
            true
        };
        let r = Literal::exec(&p, Input::Utf16(&text), 0, &mut caps, &mut (), &mut stop);
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

    #[test]
    fn flags_parse_and_letters() {
        let all = Flags::parse(&utf16("yvsmigd")).unwrap();
        assert_eq!(all.bits(), 0b1101_1111);
        assert_eq!(all.to_letters(), "dgimsvy");
        assert_eq!(Flags::parse(&[]), Some(Flags::empty()));
        assert_eq!(Flags::parse(&utf16("gg")), None, "a flag twice");
        assert_eq!(Flags::parse(&utf16("x")), None, "not a flag");
        assert_eq!(Flags::parse(&utf16("G")), None, "flags are lower case");
        assert_eq!(Flags::parse(&utf16("uv")).map(Flags::bits), Some(0x60));
        assert_eq!(Flags::from_bits(0xff).to_letters(), "dgimsuvy");
    }

    fn run(p: &str, f: &str, input: Input<'_>, start: usize) -> Option<Vec<Option<Span>>> {
        let prog = Rx0::compile(&utf16(p), Flags::parse(&utf16(f)).unwrap()).unwrap();
        let mut caps = Vec::new();
        let mut scratch = Rx0Scratch::default();
        match Rx0::exec(&prog, input, start, &mut caps, &mut scratch, &mut never) {
            ExecResult::Match => Some(caps),
            ExecResult::NoMatch => None,
            r => panic!("{r:?}"),
        }
    }

    #[test]
    fn rx0_matches_both_input_forms() {
        let m = run("(a)(x)?b", "", Input::Latin1(b"zzab"), 0).unwrap();
        assert_eq!(m, [Some((2, 4)), Some((2, 3)), None]);
        // Latin-1 above 0x7f is widened per unit (no UTF-8 decoding).
        let m = run("\u{e9}+", "", Input::Latin1(&[b'c', 0xe9, 0xe9]), 0).unwrap();
        assert_eq!(m, [Some((1, 3))]);
        let text = utf16("x\u{1F600}y");
        assert_eq!(
            run("^.", "", Input::Utf16(&text[1..]), 0).unwrap(),
            [Some((0, 1))]
        );
        assert_eq!(
            run("^.", "u", Input::Utf16(&text[1..]), 0).unwrap(),
            [Some((0, 2))]
        );
        assert_eq!(
            run("y", "", Input::Utf16(&text), 4),
            None,
            "start past the match"
        );
        assert_eq!(run("", "", Input::Utf16(&text), 4).unwrap(), [Some((4, 4))]);
        assert_eq!(run("", "", Input::Utf16(&text), 5), None, "start > length");
    }

    #[test]
    fn rx0_sticky_matches_only_at_start() {
        assert_eq!(run("a", "y", Input::Latin1(b"ba"), 0), None);
        assert_eq!(
            run("a", "y", Input::Latin1(b"ba"), 1).unwrap(),
            [Some((1, 2))]
        );
        assert_eq!(
            run("a", "g", Input::Latin1(b"ba"), 0).unwrap(),
            [Some((1, 2))]
        );
    }

    #[test]
    fn rx0_compile_errors_are_syntax_errors() {
        let e = Rx0::compile(&utf16("("), Flags::GLOBAL).unwrap_err();
        assert!(
            e.message.starts_with("Invalid regular expression: /(/g: "),
            "{}",
            e.message
        );
        let e = Rx0::compile(&utf16("a"), Flags::parse(&utf16("uv")).unwrap()).unwrap_err();
        assert_eq!(
            e.message,
            "Invalid flags supplied to RegExp constructor 'uv'"
        );
    }

    /// A raw astral character in a group name without `u`: the name is read as code points
    /// (RegExpIdentifierName is `[+UnicodeMode]`), as node v24.22.1 does
    /// (`/(?<𝒜>x)\k<𝒜>/.exec("axx")` is `["xx", "x"]` at index 1).
    #[test]
    fn rx0_astral_group_names_without_u() {
        assert_eq!(
            run("(?<\u{1d49c}>x)", "", Input::Latin1(b"x"), 0).unwrap(),
            [Some((0, 1)), Some((0, 1))]
        );
        assert_eq!(
            run(
                "(?<\u{1d49c}>x)\\k<\u{1d49c}>",
                "",
                Input::Latin1(b"axx"),
                0
            )
            .unwrap(),
            [Some((1, 3)), Some((1, 2))]
        );
        // The escaped forms of the same name, which worked before.
        assert_eq!(
            run(
                "(?<\\u{1d49c}>x)\\k<\\ud835\\udc9c>",
                "",
                Input::Latin1(b"xx"),
                0
            )
            .unwrap(),
            [Some((0, 2)), Some((0, 1))]
        );
    }

    /// `\k<𝒜>` without `u` and without any named group is the B.1.2 identity escape `k`
    /// followed by the text `<𝒜>`, whose pair stays two code units (Codex M4-P2 r2; the
    /// golden lines 529-540). Node
    /// v24.22.1: `/\k<𝒜>/.exec("k<𝒜>")` and the same with `i` are `["k<𝒜>"]` at index 0;
    /// with a named group anywhere `\k<𝒜>` is a backreference (`/\k<𝒜>(?<𝒜>y)/.exec("y")` is
    /// `["y", "y"]`, `/(?<a>x)\k<𝒜>|(?<𝒜>y)/.exec("x")` is `["x", "x", null]`) or, with no
    /// group `𝒜`, a SyntaxError ("Invalid named capture referenced").
    #[test]
    fn rx0_k_without_named_groups_is_an_identity_escape() {
        let text = utf16("k<\u{1d49c}>");
        for f in ["", "i"] {
            assert_eq!(
                run("\\k<\u{1d49c}>", f, Input::Utf16(&text), 0),
                Some(vec![Some((0, 5))]),
                "flags {f:?}"
            );
        }
        assert_eq!(
            run("\\k<\u{1d49c}>(?<\u{1d49c}>y)", "", Input::Latin1(b"y"), 0).unwrap(),
            [Some((0, 1)), Some((0, 1))]
        );
        assert_eq!(
            run(
                "(?<a>x)\\k<\u{1d49c}>|(?<\u{1d49c}>y)",
                "",
                Input::Latin1(b"x"),
                0
            )
            .unwrap(),
            [Some((0, 1)), Some((0, 1)), None]
        );
        // The text after an identity `\k` is read like the rest of the pattern: surrogate
        // escapes are code units, a class is a class. A `(?<` that is escaped or in a class
        // is no group name (node: each is `[<the whole text>]` at index 0).
        let escaped_pair = run("\\k<\\uD835\\uDC9C>", "", Input::Utf16(&text), 0);
        assert_eq!(escaped_pair, Some(vec![Some((0, 5))]));
        assert_eq!(
            run("\\k<[a]>", "", Input::Latin1(b"k<a>"), 0).unwrap(),
            [Some((0, 4))]
        );
        assert_eq!(
            run("[(?<a>x)]\\k<a>", "", Input::Latin1(b"ak<a>"), 0).unwrap(),
            [Some((0, 5))]
        );
        assert_eq!(
            run("\\(?<a>x\\)\\k<a>", "", Input::Latin1(b"<a>x)k<a>"), 0).unwrap(),
            [Some((0, 9))]
        );
        for p in ["(?<a>x)\\k<\u{1d49c}>", "\\k<\u{1d49c}>(?<a>x)"] {
            assert!(Rx0::compile(&utf16(p), Flags::empty()).is_err(), "{p}");
        }
    }

    /// A lead-surrogate escape followed by a `\u` escape that is not its trail, in `u` and
    /// `v` mode (Codex M4-P2 R1B; node v24.22.1 results, the golden lines 506-523).
    #[test]
    fn rx0_lone_lead_escape_before_another_escape() {
        for f in ["u", "v"] {
            let lead_a = [0xd800, u16::from(b'a')];
            assert_eq!(
                run("\\uD800\\u0061", f, Input::Utf16(&lead_a), 0).unwrap(),
                [Some((0, 2))]
            );
            let mut lead_text = vec![0xd800];
            lead_text.extend(utf16("0061"));
            assert_eq!(run("\\uD800\\u0061", f, Input::Utf16(&lead_text), 0), None);
            assert_eq!(
                run("\\uD800\\u{61}", f, Input::Utf16(&lead_a), 0).unwrap(),
                [Some((0, 2))]
            );
            assert_eq!(
                run("[\\uD800\\u0061]", f, Input::Latin1(b"a"), 0).unwrap(),
                [Some((0, 1))]
            );
            let leads = [0xd800, 0xd800];
            assert_eq!(
                run("\\uD800\\uD800", f, Input::Utf16(&leads), 0).unwrap(),
                [Some((0, 2))]
            );
            // Still a pair; an escaped backslash is no escape.
            let pair = utf16("\u{10000}");
            assert_eq!(
                run("\\uD800\\uDC00", f, Input::Utf16(&pair), 0).unwrap(),
                [Some((0, 2))]
            );
            assert_eq!(
                run("\\\\uD800\\u0061", f, Input::Latin1(b"\\uD800a"), 0).unwrap(),
                [Some((0, 7))]
            );
            let flags = Flags::parse(&utf16(f)).unwrap();
            assert!(Rx0::compile(&utf16("\\uD800\\u12"), flags).is_err());
        }
    }

    #[test]
    fn rx0_polls_before_the_search() {
        let prog = Rx0::compile(&utf16("a"), Flags::empty()).unwrap();
        let mut caps = Vec::new();
        let mut polls = 0;
        let mut stop = || {
            polls += 1;
            true
        };
        let mut scratch = Rx0Scratch::default();
        let r = Rx0::exec(
            &prog,
            Input::Latin1(b"a"),
            0,
            &mut caps,
            &mut scratch,
            &mut stop,
        );
        assert_eq!((r, polls), (ExecResult::Interrupted, 1));
    }

    /// Runs `p` on `input` from `start` with a poll that counts its calls and returns true
    /// at call `stop_at` (0: never). Returns the result and the poll count.
    fn run_polled(
        p: &str,
        f: &str,
        input: Input<'_>,
        start: usize,
        stop_at: u32,
    ) -> (ExecResult, u32) {
        let prog = Rx0::compile(&utf16(p), Flags::parse(&utf16(f)).unwrap()).unwrap();
        let (mut caps, mut scratch, mut polls) = (Vec::new(), Rx0Scratch::default(), 0u32);
        let mut poll = || {
            polls += 1;
            polls == stop_at
        };
        let r = Rx0::exec(&prog, input, start, &mut caps, &mut scratch, &mut poll);
        (r, polls)
    }

    /// Patch 0001: an interrupt ends an exponential search at the first poll inside it, so
    /// within one poll interval of steps (it ran 2^30 steps before the patch).
    #[test]
    fn rx0_an_interrupt_stops_a_search_within_one_poll_interval() {
        let a30 = [b'a'; 30];
        let (r, polls) = run_polled("(a+)+b", "", Input::Latin1(&a30), 0, 2);
        assert_eq!((r, polls), (ExecResult::Interrupted, 2));
        // Without an interrupt the search polls once per interval and goes on (a short run:
        // 18 a's is 2^18 steps).
        let a18 = [b'a'; 18];
        let (r, polls) = run_polled("(a+)+b", "", Input::Latin1(&a18), 0, 0);
        assert_eq!(r, ExecResult::NoMatch);
        assert!(polls > (1 << 18) / POLL_INTERVAL, "{polls} polls");
    }

    /// Patch 0001: each iteration of a single-character loop is a step; patch 0005: each scan
    /// chunk and each candidate of the UTF-16 prefix search is a step. A long run polls once
    /// per interval (before: once, before the search).
    #[test]
    fn rx0_single_char_loops_and_prefix_scans_poll() {
        let a = vec![b'a'; 100_000];
        let (r, polls) = run_polled("a+", "", Input::Latin1(&a), 0, 0);
        assert_eq!(r, ExecResult::Match);
        assert!(polls > 90, "{polls} polls");
        let mut u = vec![u16::from(b'a'); 20_000];
        u.push(0x100);
        let p = format!("{}b", "a".repeat(2000));
        let (r, polls) = run_polled(&p, "", Input::Utf16(&u), 0, 0);
        assert_eq!(r, ExecResult::NoMatch);
        assert!(polls > 15, "{polls} polls");
    }

    /// Patch 0005: the start-position scan is metered in chunks, also when each candidate
    /// fails inside it (a prefix whose mismatch is in its first 16 units) or no unit is a
    /// candidate (a first-unit set): a long subject polls, and an interrupt stops it (Codex
    /// M5-P3 r2 eng [high]; before, the scan polled once, before the search).
    #[test]
    fn rx0_rejected_scan_candidates_poll() {
        let n = 4 << 20;
        let (a, u) = (vec![b'a'; n], vec![u16::from(b'a'); n]);
        let p = format!("{}b", "a".repeat(15));
        for pat in [p.as_str(), "[bc]x"] {
            for input in [Input::Latin1(&a), Input::Utf16(&u)] {
                let (r, polls) = run_polled(pat, "", input, 0, 0);
                assert_eq!(r, ExecResult::NoMatch, "/{pat}/");
                assert!(polls >= 3, "/{pat}/: {polls} polls");
                let (r, polls) = run_polled(pat, "", input, 0, 2);
                assert_eq!((r, polls), (ExecResult::Interrupted, 2), "/{pat}/");
            }
        }
    }

    /// Patch 0005: a long literal prefix on Latin-1 text (Codex M5-P3 r3 eng [high]: a 256 KiB
    /// prefix against 4 MiB with a 'b' every 128 KiB). The scan chunk is at least 4 times the
    /// prefix, so the scan reads each unit at most 1.25 times. With 1,024-position chunks it
    /// searched the whole prefix again for each chunk. The yardstick is one `str::contains`
    /// (std is built optimized; regress here is not): about 50 times it after the fix and 700
    /// times it before, in a debug build on a64. It still polls, and an interrupt stops it.
    #[test]
    fn rx0_a_long_latin1_prefix_scan_reads_each_unit_about_once() {
        let len = 256 << 10;
        let mut text = vec![b'a'; 4 << 20];
        for p in (len / 2..text.len()).step_by(len / 2) {
            text[p] = b'b';
        }
        let pat = "a".repeat(len);
        let best = |f: &mut dyn FnMut()| {
            (0..3)
                .map(|_| {
                    let t = std::time::Instant::now();
                    f();
                    t.elapsed()
                })
                .min()
                .unwrap()
        };
        let hay = std::str::from_utf8(&text).unwrap();
        let one = best(&mut || assert!(!std::hint::black_box(hay).contains(pat.as_str())));
        let mut polls = 0;
        let scan = best(&mut || {
            let (r, p) = run_polled(&pat, "", Input::Latin1(&text), 0, 0);
            assert_eq!(r, ExecResult::NoMatch);
            polls = p;
        });
        assert!(polls >= 3, "{polls} polls");
        assert!(
            scan <= one * 150 + std::time::Duration::from_millis(20),
            "scan {scan:?}, one search {one:?}"
        );
        let (r, polls) = run_polled(&pat, "", Input::Latin1(&text), 0, 2);
        assert_eq!((r, polls), (ExecResult::Interrupted, 2));
    }

    /// Patch 0002: a sticky exec is one attempt at `start`. A sticky lexer that fails at each
    /// position of a long subject polls once per exec (before its attempt): no exec scans the
    /// rest of the subject (before the patch each one searched to the end: n^2 / 1024 polls).
    #[test]
    fn rx0_a_failing_sticky_exec_does_not_scan() {
        let text = vec![u16::from(b'a'); 20_000];
        let prog = Rx0::compile(&utf16("b|ab?c"), Flags::STICKY).unwrap();
        let (mut caps, mut scratch, mut polls) = (Vec::new(), Rx0Scratch::default(), 0usize);
        let mut poll = || {
            polls += 1;
            false
        };
        for start in 0..text.len() {
            let r = Rx0::exec(
                &prog,
                Input::Utf16(&text),
                start,
                &mut caps,
                &mut scratch,
                &mut poll,
            );
            assert_eq!(r, ExecResult::NoMatch);
        }
        assert_eq!(polls, text.len());
        // A sticky match at `start`, and none at a later position.
        assert_eq!(
            run("ab?c", "y", Input::Latin1(b"xabc"), 1).unwrap(),
            [Some((1, 4))]
        );
        assert_eq!(run("ab?c", "y", Input::Latin1(b"xabc"), 0), None);
    }

    /// Patch 0003: `\k<x>` of a name that two groups share matches the one that took part
    /// (test262 duplicate-names-exec.js; node v24.22.0).
    #[test]
    fn rx0_duplicate_names_in_a_backreference() {
        let p = "(?:(?<x>a)|(?<x>b))\\k<x>";
        assert_eq!(
            run(p, "", Input::Latin1(b"bb"), 0).unwrap(),
            [Some((0, 2)), None, Some((0, 1))]
        );
        assert_eq!(
            run(p, "", Input::Latin1(b"aa"), 0).unwrap(),
            [Some((0, 2)), Some((0, 1)), None]
        );
        assert_eq!(run(p, "", Input::Latin1(b"ab"), 0), None);
        // [\k] with a group name is a SyntaxError without u (B.1.2; node "Invalid escape").
        for p in ["[\\k<a>](?<a>x)", "(?<a>x)[\\k<a>]"] {
            assert!(Rx0::compile(&utf16(p), Flags::empty()).is_err(), "{p}");
        }
    }

    /// Patch 0005: a Latin-1 subject is read in place, with the same results as its UTF-16
    /// form, also for case pairs that leave Latin-1 (U+00FF, U+0178) and start predicates.
    #[test]
    fn rx0_latin1_equals_utf16() {
        let cases = [
            ("\\u0178", "i", &[b'a', 0xff][..]),
            ("q{2,}x|\\d", "", b"qqx9"),
            ("^b", "m", b"a\nb"),
            ("[\\u00e0-\\u00ff]+", "i", &[b'A', 0xc9, 0xe9]),
            ("(?<=\\u00e9)b", "", &[0xe9, b'b']),
        ];
        for (p, f, bytes) in cases {
            let units: Vec<u16> = bytes.iter().map(|&b| u16::from(b)).collect();
            for start in 0..=bytes.len() {
                assert_eq!(
                    run(p, f, Input::Latin1(bytes), start),
                    run(p, f, Input::Utf16(&units), start),
                    "/{p}/{f} from {start}"
                );
            }
        }
        assert_eq!(
            run("\\u0178", "i", Input::Latin1(&[b'a', 0xff]), 0).unwrap(),
            [Some((1, 2))]
        );
    }
}
