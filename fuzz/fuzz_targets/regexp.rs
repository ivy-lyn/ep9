//! RegExp pattern -> the RX1 parser (ep9_rx1) and the front end's early-error check
//! (ep9_frontend, oxc_regular_expression) must agree on "valid or not"; RX1 must not panic.
//!
//! Input: byte 0 is the flags (ep9_rx1::Flags bits: d g i m s u v y = bits 0-7), the rest is
//! the pattern as UTF-8 text. Parse only in M7-P4 (decisions-log M6-ORCH-300): the exec part
//! (RX1 compile + exec against the shipped RX0) is route RX1-FUZZ-EXEC (RX1-1, M7-P5).
//!
//! ```text
//! flags byte + text
//!   u and v together              -> skip (a flags error, before any pattern parse)
//!   text is not a RegularExpressionBody (line terminator, an unescaped / outside a class,
//!   a trailing \, empty, starts with *) -> skip (no literal can hold it)
//!   oxc: check_early_errors("/<text>/<flags>") as a sloppy script
//!        Ok -> valid; Syntax -> invalid; Range (the depth caps) -> skip
//!   RX1: ep9_rx1::parse(text as UTF-16, flags)
//!        Ok -> valid; StackOverflow (the nesting cap) -> skip; other error -> invalid
//!   valid != valid -> panic (a crash: one of the two parsers is wrong), except the known
//!   front-end false accepts and false rejects below
//! ```
//!
//! Known front-end false accepts (oxc_regular_expression 0.155; node v26.13.0 and RX1 refuse
//! each, M7-P4 RX1-0 report), not counted as crashes (the rx-dupnames case, a group inside a
//! group of the same name such as `/(?<a>x|(?<a>y))/`, is refused by oxc 0.157.0, M7-P4
//! OXCBUMP, so the target counts a disagreement on it as a crash):
//! - `\c` with no control letter inside a class in Unicode mode, for example `/[\c]/u`. RX1
//!   says InvalidUnicodeEscape at the `c`.
//! - `\A`, `\z` or `\Z` in Unicode mode, for example `/\z/u` (oxc_regular_expression 0.155
//!   and 0.157 parse them as the assertions of the TC39 RegExp buffer boundaries proposal).
//!   ECMA-262 has no such Assertion: in Unicode mode `\z` is an AtomEscape, and
//!   IdentityEscape[+UnicodeMode] is only SyntaxCharacter or `/` (22.2.1). RX1 says
//!   InvalidEscape at the letter (fuzz smoke of CI run 42552778227).
//!
//! Known front-end false reject (oxc_regular_expression 0.157; node v26.13.0 and RX1 accept
//! it), not counted as a crash:
//! - a braced quantifier number above 2^53 - 1, for example `/a{9007199254740992}/`. oxc says
//!   "Number is too large in braced quantifier". ECMA-262 22.2.1.1 has no such early error
//!   (only min > max), and RepeatMatcher takes any integer max; V8 clamps the number.
#![no_main]

use ep9_frontend::{CompileError, EarlyErrors, SourceOpts, Strictness};
use ep9_rx1::{Error, Flags, ParseError};

/// True for a known front-end false accept (the module doc), given RX1's error.
fn known_front_end_false_accept(units: &[u16], flags: Flags, e: &ParseError) -> bool {
    let escaped = |letters: &[u8]| {
        e.pos >= 1
            && units
                .get(e.pos)
                .is_some_and(|&c| letters.iter().any(|&l| c == u16::from(l)))
            && units[e.pos - 1] == u16::from(b'\\')
    };
    match e.error {
        Error::InvalidUnicodeEscape => escaped(b"c"),
        Error::InvalidEscape => flags.either_unicode() && escaped(b"AzZ"),
        _ => false,
    }
}

/// True for the known front-end false reject (the module doc): every oxc error is the
/// quantifier size error.
fn known_front_end_false_reject(errors: &EarlyErrors) -> bool {
    errors.as_slice().iter().all(|e| {
        e.message
            .contains("Number is too large in braced quantifier")
    })
}

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
    if !is_literal_body(text) {
        return;
    }
    let source = format!("/{text}/{}", flags.to_letters());
    let oxc_errors =
        match ep9_frontend::check_early_errors(&source, SourceOpts::script(Strictness::Sloppy)) {
            Ok(()) => None,
            Err(CompileError::Syntax(errors)) => Some(errors),
            Err(CompileError::Range(_)) => return,
        };
    let oxc_valid = oxc_errors.is_none();
    let units: Vec<u16> = text.encode_utf16().collect();
    let rx1_valid = match ep9_rx1::parse(&units, flags) {
        Ok(_) => true,
        Err(e) if e.error.is_stack_overflow() => return,
        Err(e) if oxc_valid && known_front_end_false_accept(&units, flags, &e) => return,
        Err(_) => false,
    };
    if rx1_valid
        && oxc_errors
            .as_ref()
            .is_some_and(known_front_end_false_reject)
    {
        return;
    }
    assert!(
        rx1_valid == oxc_valid,
        "RX1 and the front end disagree: rx1 valid {rx1_valid}, oxc valid {oxc_valid}"
    );
});
