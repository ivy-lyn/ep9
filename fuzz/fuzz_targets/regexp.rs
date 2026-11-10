//! RegExp pattern -> three checks: the RX1 parser (ep9_rx1) and the front end's early-error
//! check (ep9_frontend, oxc_regular_expression) agree on "valid or not"; RX1 and the shipped
//! RX0 (ep9_regexp, regress) agree on "valid or not" and on the group count; and on a pattern
//! that is one class, RX1's code point set (port step 2) is the set that RX0 matches. Nothing
//! may panic. Route RX1-FUZZ-EXEC (M7-P5): RX1 has no matcher yet (port steps 3-5), so the
//! exec part is the RX0 exec of every pattern (a step budget, the match spans) and the class
//! set differential.
//!
//! Input: byte 0 is the flags (ep9_rx1::Flags and ep9_regexp::Flags bits: d g i m s u v y =
//! bits 0-7), the rest is the pattern as UTF-8 text.
//!
//! ```text
//! flags byte + text
//!   u and v together              -> skip (a flags error, before any pattern parse)
//!   RX1: ep9_rx1::parse(text as UTF-16, flags)
//!        Ok -> valid; StackOverflow (the nesting cap) -> skip; other error -> invalid
//!   1. front end (only if the text is a RegularExpressionBody: no line terminator, no
//!      unescaped / outside a class, no trailing \, not empty, no * first)
//!        check_early_errors("/<text>/<flags>") as a sloppy script
//!        Ok -> valid; Syntax -> invalid; Range (the depth caps) -> no check
//!        valid != RX1 valid -> panic
//!   2. RX0: ep9_regexp::Rx0::compile(text, flags)
//!        valid != RX1 valid -> panic, except the known RX0 differences below
//!        both valid: exec on the pattern text as the subject, at most POLLS polls;
//!        a match: one span per group (RX1 capture_count + 1), each inside the subject
//!   3. the pattern is one class (Tree::ClassRanges, ClassSetOperand, ClassSetExpression)
//!        RX1 set (class_ranges / class_set) against RX0 "^(?:<text>)$" (flags i s u v) on
//!        probe characters (the pattern's characters, their case equivalents, the RX1 range
//!        ends and a fixed list) and on the class strings
//!        a difference -> panic, except the known RX0 differences below
//! ```
//!
//! Known RX0 differences (regress 0.12.0 with the ep9 patches; RX1 follows ECMA-262 on each,
//! and node v26.13.0 agrees with RX1 except where a row says that V8 does as regress), found
//! by a replay of 119,062 inputs (the seeds, the test262 sources and grammar-made patterns),
//! not counted as crashes:
//! - Validity: a braced quantifier `{n,m}` with n > m where regress caps both numbers to one
//!   value, for example `/a{99999999999999999999,99999999999999999998}/`. RX1 says
//!   RangeOutOfOrder (ECMA-262 22.2.1.1 compares the MVs); regress (and V8) take it. The front
//!   end refuses it too (M7-P5 FE-RX-BIGQUANT).
//! - Validity: a negated `v` class that may contain strings, for example `/[^\q{ab}]/v`. RX1
//!   says NegatedCharacterClassWithStrings (ECMA-262 22.2.1.1, MayContainStrings); regress
//!   takes it.
//! - Validity: a `\u` escape of `>` in a group name, for example `/(?<a\u003e>x)/`. V8 and
//!   regress end the name at the escape; RX1 refuses it (U+003E is not an IdentifierPartChar,
//!   ECMA-262 22.2.1.1; ep9_rx1 crate doc, "Known gaps"). The target skips any
//!   InvalidCaptureGroupName of a pattern with `\u`.
//! - Validity: a quantified `\b` or `\B`, for example `/\b*/`. RX1 says NothingToRepeat (a
//!   Term quantifies only an Atom or, without `u` and `v`, a QuantifiableAssertion, a
//!   look-ahead; ECMA-262 22.2.1 and B.1.2); regress takes it in every mode. The target
//!   skips any NothingToRepeat of a pattern with `\b` or `\B`.
//! - Validity: an unescaped `[` or `]` inside `\q{..}` of a `v` class, for example
//!   `/[\q{?[}]/v` and `/[\q{a]}]/v`. RX1 says InvalidCharacterInClass (ECMA-262 22.2.1: a
//!   ClassSetCharacter is not a ClassSetSyntaxCharacter), and node v26.13.0 and ep9's front end
//!   (a literal) refuse it; regress takes it, so `new RegExp` in ep9 takes it (the other
//!   ClassSetSyntaxCharacters `( ) { / -` there regress refuses too). The target skips an RX1
//!   InvalidCharacterInClass with `v` only when the error position is such a `[` or `]`.
//! - Validity: a `]` right after `&&`, `--` or the `-` of a range in a `v` class, for example
//!   `/[[.]&&]]/v`, `/[[a]--]]/v`, `/[A-]]/v`. RX1 says InvalidCharacterInClass (an operator
//!   needs a ClassSetOperand and a ClassSetRange a ClassSetCharacter, and `]` is a
//!   ClassSetSyntaxCharacter; ECMA-262 22.2.1), and node and ep9's front end refuse it; regress
//!   takes the `]` as the operand or as the range end (U+005D: so /[A-]]/v is a range and
//!   /[a-]]/v is out of order) when one more `]` closes the class. The target skips an RX1
//!   InvalidCharacterInClass with `v` only when the error position is such a `]`. In the same
//!   way regress takes a third `&` as the right operand of `&&` just before the class end, for
//!   example `/[a&&&]/v` (ECMA-262 22.2.1: ClassIntersection has [lookahead is not &] after `&&`);
//!   the target skips it only when the error position is that `&` and a `]` follows it.
//! - Validity: a `\u` escape with a `+` sign before its hex digits, with `u` or `v` (for
//!   example `/[\u+1F6]/u`, also `\u{+1F600}` and outside a class) and in a group name in
//!   every mode (for example `/(?<a\u+041>x)/`, also in `\k<..>`). RX1 says
//!   InvalidUnicodeEscape (ECMA-262 22.2.1: HexDigits have no sign), and node and ep9's front
//!   end refuse it; regress takes `+1F6` as the 4 hex digits (a sign that Rust's
//!   `u32::from_str_radix` accepts). The target skips an RX1 InvalidUnicodeEscape only when the
//!   error position is just after such a `\u` (found by the M7-P5 fixer r1 and r2 10-minute
//!   runs, forms `/[\u+1F600}-\u14F6{P{Uu}]/u` and `/..(?<a\u+03e:>x)\k<a>/gs`).
//! - The proof of the 3 rows above (fixer r2, node v26.13.0; in ep9 `new RegExp` is RX0 and
//!   the literal is the front end; "Syntax" = a SyntaxError):
//!
//! ```text
//!   pattern          node: RegExp, literal   ep9: RegExp, literal   RX1 error, position
//!   /[\q{?[}]/v      Syntax, Syntax          ok, Syntax             InvalidCharacterInClass, 5
//!   /[\q{a]}]/v      Syntax, Syntax          ok, Syntax             InvalidCharacterInClass, 5
//!   /[[.]&&]]/v      Syntax, Syntax          ok, Syntax             InvalidCharacterInClass, 6
//!   /[[a]--]]/v      Syntax, Syntax          ok, Syntax             InvalidCharacterInClass, 6
//!   /[A-]]/v         Syntax, Syntax          ok, Syntax             InvalidCharacterInClass, 3
//!   /[A-]x]/v        Syntax, Syntax          ok, Syntax             InvalidCharacterInClass, 3
//!   /[a&&&]/v        Syntax, Syntax          ok, Syntax             InvalidCharacterInClass, 4
//!   /[[a]&&&]/v      Syntax, Syntax          ok, Syntax             InvalidCharacterInClass, 6
//!   /[\u+1F6]/u      Syntax, Syntax          ok, Syntax             InvalidUnicodeEscape, 3
//!   /\u{+1F600}/u    Syntax, Syntax          ok, Syntax             InvalidUnicodeEscape, 2
//!   /(?<a\u+041>x)/  Syntax, Syntax          ok, Syntax             InvalidUnicodeEscape, 6
//!   /(?<a\u{+41}>x)/ Syntax, Syntax          ok, Syntax             InvalidUnicodeEscape, 6
//! ```
//! - Validity: regress's `v` parser refuses some valid classes, for example `/[.$]/v` (two
//!   different ClassSetReservedPunctuator characters; only a doubled one is a
//!   ClassSetReservedDoublePunctuator) and `/[^ß&&\p{RGI_Emoji}]/v` (an intersection with
//!   one operand that has no strings cannot contain strings, ECMA-262 MayContainStrings). The
//!   target does not compare an RX0 refusal with `v`; the front-end check still covers RX1
//!   there.
//! - Validity: groups nested deeper than regress's cap (MAX_NESTING_DEPTH), for example 256
//!   `(?:` around `a`. regress says "Regular expression is too deeply nested"; RX1 takes up
//!   to ep9_rx1::MAX_NESTING levels (node takes 10,000).
//! - Sets: `\W` in a class with `i` and `u` or `v`, for example `/[\W]/iu`. regress also
//!   matches k, s, K, S, U+017F and U+212A (node and RX1: no match; with `u`+`i`, U+017F and
//!   U+212A are word characters, ECMA-262 WordCharacters).
//! - Sets: `[\b]` with `v`. regress does not match U+0008 (node and RX1 match it;
//!   ClassSetCharacter has `\b`, ECMA-262 22.2.1).
//! - Sets: `\P{..}` with `v` and `i`, for example `/[\P{Lu}]/iv`. regress matches A (node and
//!   RX1 do not: with `v`+`i`, CharacterComplement takes only the characters c with
//!   scf(c) = c, ECMA-262 AllCharacters).
//! - Sets: `&&` and `--` with `v` and `i`, for example `/[[a]&&[A]]/iv`. regress takes the
//!   intersection or difference before the case folding, so it matches nothing here (node
//!   and RX1 match a and A: the operands are MaybeSimpleCaseFolding sets, ECMA-262).
//! - Sets: a nested negated class with `v` and `i`, for example `/[[^s]]/iv`. regress
//!   matches S (node and RX1 do not; CharacterComplement under `v`+`i`, as for `\P` above).
//! - Sets: a negated class and `\q{..}` with `v`, for example `/[^\q{K}]/v`. regress matches
//!   K and does not match a (node and RX1: the other way round).
//! - Sets: `\c` before a `\` in a class without `u` and `v`, for example
//!   `/[\c\u{10400}]/`. regress does not match `\` (node and RX1 do: Annex B
//!   ClassAtomNoDash `\` [lookahead = c], B.1.2).
//! - Sets: a single `&` (not `&&`, not escaped) in a `v` class, for example `/[a&b]/v` and
//!   `/[\!&]/v`. regress drops the ClassSetOperand just before the `&` (a character, an
//!   escape, a nested class, `\q{..}`, `\w`, `\p{..}`; not a range), so it does not match `a`
//!   and `!` there (node and RX1 do: a single `&` is a ClassSetCharacter, ECMA-262 22.2.1;
//!   found by the M7-P5 fixer r1 and r2 runs, `/[\!&function \#&]/v`, `/[L&Po\!\#a]/v`).
//!   Proof (fixer r2, node v26.13.0; ep9 `new RegExp` = RX0; each on `^(?:<class>)$`):
//!
//! ```text
//!   class            subject   node    ep9
//!   /[a&b]/v         "a"       true    false
//!   /[L&Po\!\#a]/v   "L"       true    false
//!   /[\!&]/v         "!"       true    false
//!   /[[a]&b]/v       "a"       true    false
//!   /[\q{a}&b]/v     "a"       true    false
//!   /[\w&]/v         "a"       true    false
//!   /[^a&]/v         "a"       false   true
//!   /[a-c&]/v        "a"       true    true
//! ```
//! - Sets: the empty string of a `v` class, for example `/^[\q{}]$/v` on "". regress never
//!   matches it (node and RX1 do; ClassString can be empty, ECMA-262 22.2.1).
//!
//! The target skips the set check of any class text with the escapes above under those
//! flags, and the empty class string.
//!
//! The front-end false accepts and false reject of M7-P4 (`\c` with no letter in a class,
//! `\A`, `\z`, `\Z` in Unicode mode; a braced number above 2^53 - 1) are fixed in the front
//! end in M7-P5 (regex_check.rs, routes FE-RX-BUFBOUND and FE-RX-BIGQUANT), so a
//! disagreement on them is a crash now.
#![no_main]

use ep9_frontend::{CompileError, SourceOpts, Strictness};
use ep9_regexp::{Backend, ExecResult, Input, Rx0, Rx0Scratch};
use ep9_rx1::{
    CharacterRange, Error, Flags, ParseError, Pattern, Tree, case_independent_letters,
    class_ranges, class_set, simple_fold,
};

/// The end of regress's error for groups nested deeper than its cap (a known RX0 difference,
/// the module doc).
const RX0_TOO_DEEP: &str = "Regular expression is too deeply nested";

/// The polls of one RX0 exec (each after `ep9_regexp::POLL_INTERVAL` steps) before it stops.
const POLLS: u32 = 64;

/// The probe characters of the set check that every class gets.
const FIXED_PROBES: [u32; 26] = [
    0, 0x0a, 0x0d, 0x20, 0x30, 0x41, 0x4b, 0x53, 0x5f, 0x61, 0x6b, 0x73, 0xdf, 0x130, 0x131, 0x17f,
    0x3c2, 0x1e9e, 0x2028, 0x212a, 0xd800, 0xdc00, 0xffff, 0x10000, 0x1f600, 0x10ffff,
];

/// The most pattern characters and RX1 ranges that give probes (a bound on the work of one
/// input).
const MAX_PROBE_SOURCES: usize = 48;

/// True if `text` is a RegularExpressionBody (ECMA-262 12.9.5): `/<text>/` lexes as one
/// regexp literal.
fn is_literal_body(text: &str) -> bool {
    let line_terminator = |c: char| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
    if text.is_empty() || text.starts_with('*') {
        return false;
    }
    let mut chars = text.chars();
    let mut in_class = false;
    while let Some(c) = chars.next() {
        if line_terminator(c) {
            return false;
        }
        match c {
            '\\' => match chars.next() {
                Some(d) if !line_terminator(d) => {}
                _ => return false,
            },
            '[' => in_class = true,
            ']' => in_class = false,
            '/' if !in_class => return false,
            _ => {}
        }
    }
    true
}

/// Check 1: the front end and RX1 agree on "valid or not".
fn check_front_end(text: &str, flags: Flags, rx1_valid: bool) {
    if !is_literal_body(text) {
        return;
    }
    let source = format!("/{text}/{}", flags.to_letters());
    let oxc_valid =
        match ep9_frontend::check_early_errors(&source, SourceOpts::script(Strictness::Sloppy)) {
            Ok(()) => true,
            Err(CompileError::Syntax(_)) => false,
            Err(CompileError::Range(_)) => return,
        };
    assert!(
        rx1_valid == oxc_valid,
        "RX1 and the front end disagree: rx1 valid {rx1_valid}, oxc valid {oxc_valid}"
    );
}

/// One RX0 exec of `program` on `subject` from 0, stopped after [`POLLS`] polls.
fn rx0_exec(
    program: &<Rx0 as Backend>::Program,
    subject: &[u16],
    caps: &mut Vec<Option<(usize, usize)>>,
    scratch: &mut Rx0Scratch,
) -> ExecResult {
    let mut polls = 0_u32;
    let mut poll = || {
        polls += 1;
        polls > POLLS
    };
    Rx0::exec(program, Input::Utf16(subject), 0, caps, scratch, &mut poll)
}

/// Check 2, the exec part: RX0 runs the pattern on its own text.
fn check_rx0_exec(program: &<Rx0 as Backend>::Program, units: &[u16], rx1: &Pattern) {
    let mut caps = Vec::new();
    let mut scratch = Rx0Scratch::default();
    if rx0_exec(program, units, &mut caps, &mut scratch) != ExecResult::Match {
        return;
    }
    assert!(
        caps.len() == rx1.capture_count as usize + 1,
        "RX0 gives {} spans, RX1 counts {} groups",
        caps.len(),
        rx1.capture_count
    );
    assert!(caps[0].is_some(), "a match without a whole-match span");
    for span in caps.iter().flatten() {
        assert!(
            span.0 <= span.1 && span.1 <= units.len(),
            "a span outside the subject"
        );
    }
}

/// The one class of `tree`, if the pattern is one class.
fn single_class(tree: &Tree) -> Option<&Tree> {
    match tree {
        Tree::ClassRanges(_) | Tree::ClassSetOperand(_) | Tree::ClassSetExpression(_) => Some(tree),
        Tree::Text(v) | Tree::Alternative(v) | Tree::Disjunction(v) if v.len() == 1 => {
            single_class(&v[0])
        }
        _ => None,
    }
}

/// `c` as UTF-16 code units (a code unit stays one unit, a lone surrogate too).
fn utf16(c: u32, out: &mut Vec<u16>) {
    let mut buf = [0_u16; 2];
    match char::from_u32(c) {
        Some(ch) => out.extend_from_slice(ch.encode_utf16(&mut buf)),
        None => out.push(c as u16),
    }
}

/// The probe characters of a class: [`FIXED_PROBES`], each character of the pattern with its
/// neighbours and case equivalents, and the ends of the RX1 ranges with their neighbours.
fn probes(units: &[u16], ranges: &[CharacterRange], flags: Flags) -> Vec<u32> {
    let unicode = flags.either_unicode();
    let max = if unicode { 0x10_ffff } else { 0xffff };
    let mut out: Vec<u32> = FIXED_PROBES.to_vec();
    let chars: Vec<u32> = if unicode {
        char::decode_utf16(units.iter().copied())
            .map(|r| r.map_or_else(|e| u32::from(e.unpaired_surrogate()), u32::from))
            .collect()
    } else {
        units.iter().map(|&u| u32::from(u)).collect()
    };
    for &c in chars.iter().take(MAX_PROBE_SOURCES) {
        out.extend([c.saturating_sub(1), c, c + 1, simple_fold(c)]);
        if let Ok(u) = u16::try_from(c) {
            out.extend(
                case_independent_letters(u, unicode, false)
                    .into_iter()
                    .map(u32::from),
            );
        }
    }
    for r in ranges.iter().take(MAX_PROBE_SOURCES) {
        out.extend([r.from().saturating_sub(1), r.from(), r.to(), r.to() + 1]);
    }
    out.retain(|&c| c <= max);
    out.sort_unstable();
    out.dedup();
    out
}

/// True for an RX1 error where RX0 is known to take the pattern (the module doc). The `v` class
/// rows and the `\u` row match only their proven form at the RX1 error position, so an RX1
/// false reject of a valid pattern is still a crash. Without `u` and `v` the `\u` row needs a
/// group name too (there `\u` is always a RegExpUnicodeEscapeSequence; elsewhere `\u+` is the
/// identity escape `u` and a quantifier).
fn known_rx0_false_accept(e: &ParseError, units: &[u16], flags: Flags) -> bool {
    match e.error {
        Error::RangeOutOfOrder | Error::NegatedCharacterClassWithStrings => true,
        Error::InvalidCaptureGroupName => has_escape(units, b'u'),
        Error::InvalidCharacterInClass if flags.unicode_sets() => {
            bracket_in_class_string(units, e.pos)
                || bracket_after_set_operator(units, e.pos)
                || amp_after_intersection(units, e.pos)
        }
        Error::InvalidUnicodeEscape => {
            plus_sign_hex_escape(units, e.pos)
                && (flags.either_unicode() || in_group_name(units, e.pos - 2))
        }
        _ => false,
    }
}

/// True if `units[i]` is the ASCII character `c`.
fn unit_is(units: &[u16], i: usize, c: u8) -> bool {
    units.get(i) == Some(&u16::from(c))
}

/// True if `units[pos]` is an unescaped `[` or `]` inside the body of a `\q{..}` (from `\q{`
/// to the first unescaped `}`).
fn bracket_in_class_string(units: &[u16], pos: usize) -> bool {
    if !(unit_is(units, pos, b'[') || unit_is(units, pos, b']')) {
        return false;
    }
    let (mut i, mut in_q) = (0, false);
    while i < pos {
        if unit_is(units, i, b'\\') {
            if !in_q && unit_is(units, i + 1, b'q') && unit_is(units, i + 2, b'{') {
                in_q = true;
                i += 3;
            } else {
                i += 2;
            }
            continue;
        }
        if in_q && unit_is(units, i, b'}') {
            in_q = false;
        }
        i += 1;
    }
    // i > pos: the unit at pos is the escaped one of a `\`.
    in_q && i == pos
}

/// True if `units[pos]` is a `]` right after an unescaped `&&` or `-` (the `-` of a range or
/// the second one of `--`).
fn bracket_after_set_operator(units: &[u16], pos: usize) -> bool {
    let amp_amp = pos >= 2
        && unit_is(units, pos - 1, b'&')
        && unit_is(units, pos - 2, b'&')
        && not_escaped(units, pos - 2);
    let dash = pos >= 1 && unit_is(units, pos - 1, b'-') && not_escaped(units, pos - 1);
    unit_is(units, pos, b']') && (amp_amp || dash)
}

/// True if `units[pos]` is a `&` right after an unescaped `&&` and just before a `]`.
fn amp_after_intersection(units: &[u16], pos: usize) -> bool {
    pos >= 2
        && unit_is(units, pos, b'&')
        && unit_is(units, pos + 1, b']')
        && unit_is(units, pos - 1, b'&')
        && unit_is(units, pos - 2, b'&')
        && not_escaped(units, pos - 2)
}

/// True if `units[pos]` is not the escaped unit of a `\` (a scan from the start).
fn not_escaped(units: &[u16], pos: usize) -> bool {
    let mut i = 0;
    while i < pos {
        i += if unit_is(units, i, b'\\') { 2 } else { 1 };
    }
    i == pos
}

/// True if `units[i]` is inside a group name: after `(?<` or `\k<` (not a look-behind `(?<=`,
/// `(?<!`) with no `>` between.
fn in_group_name(units: &[u16], i: usize) -> bool {
    let mut j = i;
    while j > 0 {
        j -= 1;
        if unit_is(units, j, b'>') {
            return false;
        }
        if unit_is(units, j, b'<') {
            let opener = j >= 2
                && ((unit_is(units, j - 2, b'(') && unit_is(units, j - 1, b'?'))
                    || (unit_is(units, j - 2, b'\\') && unit_is(units, j - 1, b'k')));
            return opener && !unit_is(units, j + 1, b'=') && !unit_is(units, j + 1, b'!');
        }
    }
    false
}

/// True if `pos` is just after a `\u` and starts `+` or `{+` (a sign before the hex digits).
fn plus_sign_hex_escape(units: &[u16], pos: usize) -> bool {
    pos >= 2
        && unit_is(units, pos - 2, b'\\')
        && unit_is(units, pos - 1, b'u')
        && (unit_is(units, pos, b'+')
            || (unit_is(units, pos, b'{') && unit_is(units, pos + 1, b'+')))
}

/// True if the class text has `\<letter>`. A `\\<letter>` counts too: skipping more classes
/// is safe.
fn has_escape(units: &[u16], letter: u8) -> bool {
    units
        .windows(2)
        .any(|w| w[0] == u16::from(b'\\') && w[1] == u16::from(letter))
}

/// True if the class text has `&&` or `--` (also escaped ones: skipping more is safe).
fn has_set_operator(units: &[u16]) -> bool {
    units
        .windows(2)
        .any(|w| w[0] == w[1] && (w[0] == u16::from(b'&') || w[0] == u16::from(b'-')))
}

/// True if the text has `\b` or `\B` (also after a `\\`: skipping more is safe).
fn has_boundary(units: &[u16]) -> bool {
    has_escape(units, b'b') || has_escape(units, b'B')
}

/// True if the class text has a `[^` after its first code unit (a nested negated class).
fn has_nested_negation(units: &[u16]) -> bool {
    units
        .windows(2)
        .skip(1)
        .any(|w| w[0] == u16::from(b'[') && w[1] == u16::from(b'^'))
}

/// True if the class text has a `[^` anywhere.
fn has_negation(units: &[u16]) -> bool {
    units.starts_with(&[u16::from(b'['), u16::from(b'^')]) || has_nested_negation(units)
}

/// True if the class text has `\c` before a `\`.
fn has_control_before_escape(units: &[u16]) -> bool {
    units
        .windows(3)
        .any(|w| w[0] == u16::from(b'\\') && w[1] == u16::from(b'c') && w[2] == u16::from(b'\\'))
}

/// True for a class with a known RX0 set difference (the module doc).
fn known_rx0_set_difference(units: &[u16], flags: Flags) -> bool {
    let (i, u, v) = (
        flags.ignore_case(),
        flags.either_unicode(),
        flags.unicode_sets(),
    );
    (i && u && has_escape(units, b'W'))
        || (v && has_escape(units, b'b'))
        || (v && i && (has_escape(units, b'P') || has_set_operator(units)))
        || (v && i && has_nested_negation(units))
        || (v && has_negation(units) && has_escape(units, b'q'))
        || (!u && has_control_before_escape(units))
        || (v && has_single_amp(units))
}

/// True if the class text has a `&` that is not escaped and has no `&` next to it.
fn has_single_amp(units: &[u16]) -> bool {
    let mut i = 0;
    while i < units.len() {
        if unit_is(units, i, b'\\') {
            i += 2;
        } else if unit_is(units, i, b'&') {
            let run = units[i..]
                .iter()
                .take_while(|&&c| c == u16::from(b'&'))
                .count();
            if run == 1 {
                return true;
            }
            i += run;
        } else {
            i += 1;
        }
    }
    false
}

/// Check 3: on a one-class pattern, the RX1 set is the RX0 set.
fn check_class_set(units: &[u16], flags: Flags, rx1: &Pattern) {
    let Some(class) = single_class(&rx1.tree) else {
        return;
    };
    if known_rx0_set_difference(units, flags) {
        return;
    }
    // The flags that change what a class matches; `^(?:...)$` must not see `m` or `y`.
    let set_bits = flags.bits()
        & (Flags::IGNORE_CASE
            .union(Flags::DOT_ALL)
            .union(Flags::UNICODE)
            .union(Flags::UNICODE_SETS))
        .bits();
    let mut anchored: Vec<u16> = "^(?:".encode_utf16().collect();
    anchored.extend_from_slice(units);
    anchored.extend(")$".encode_utf16());
    let Ok(program) = Rx0::compile(&anchored, ep9_regexp::Flags::from_bits(set_bits)) else {
        panic!("RX0 compiles the class but not ^(?:class)$");
    };
    let rx1_flags = Flags::from_bits(set_bits);
    let (ranges, negated, strings) = match class {
        Tree::ClassRanges(cr) => {
            let set = class_ranges(cr, rx1_flags, false);
            (set.ranges, set.negated, Vec::new())
        }
        _ => {
            let Some(set) = class_set(class, rx1_flags) else {
                return;
            };
            let strings: Vec<Vec<u32>> = set.strings.keys().map(|k| k.0.to_vec()).collect();
            (set.ranges, false, strings)
        }
    };
    let mut caps = Vec::new();
    let mut scratch = Rx0Scratch::default();
    let mut subject = Vec::with_capacity(4);
    let mut rx0_matches =
        |subject: &[u16]| match rx0_exec(&program, subject, &mut caps, &mut scratch) {
            ExecResult::Match => Some(true),
            ExecResult::NoMatch => Some(false),
            ExecResult::Interrupted | ExecResult::BudgetExceeded => None,
        };
    for c in probes(units, &ranges, flags) {
        subject.clear();
        utf16(c, &mut subject);
        let rx1_match = ep9_rx1::contains(&ranges, c) != negated;
        let Some(rx0_match) = rx0_matches(&subject) else {
            continue;
        };
        assert!(
            rx1_match == rx0_match,
            "the class sets differ at U+{c:04X}: rx1 {rx1_match}, rx0 {rx0_match}"
        );
    }
    // The empty string is a known RX0 difference (the module doc).
    for s in strings
        .iter()
        .filter(|s| !s.is_empty())
        .take(MAX_PROBE_SOURCES)
    {
        subject.clear();
        for &c in s {
            utf16(c, &mut subject);
        }
        if rx0_matches(&subject) == Some(false) {
            panic!("RX0 does not match a string of the RX1 class set");
        }
    }
}

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&bits, rest)) = data.split_first() else {
        return;
    };
    let flags = Flags::from_bits(bits);
    if flags.contains(Flags::UNICODE.union(Flags::UNICODE_SETS)) {
        return;
    }
    let Ok(text) = std::str::from_utf8(rest) else {
        return;
    };
    let units: Vec<u16> = text.encode_utf16().collect();
    let rx1 = match ep9_rx1::parse(&units, flags) {
        Ok(p) => Ok(p),
        Err(e) if e.error.is_stack_overflow() => return,
        Err(e) => Err(e),
    };
    check_front_end(text, flags, rx1.is_ok());
    let rx0 = Rx0::compile(&units, ep9_regexp::Flags::from_bits(bits));
    match (&rx1, &rx0) {
        (Ok(p), Ok(program)) => {
            check_rx0_exec(program, &units, p);
            check_class_set(&units, flags, p);
        }
        (Err(e), Ok(_)) if known_rx0_false_accept(e, &units, flags) => {}
        (Ok(_), Err(e)) if e.message.ends_with(RX0_TOO_DEEP) => {}
        (Ok(_), Err(_)) if flags.unicode_sets() => {}
        (Err(e), Ok(_)) if e.error == Error::NothingToRepeat && has_boundary(&units) => {}
        (Err(_), Err(_)) => {}
        (rx1, rx0) => panic!(
            "RX1 and RX0 disagree: rx1 valid {}, rx0 valid {}",
            rx1.is_ok(),
            rx0.is_ok()
        ),
    }
});
