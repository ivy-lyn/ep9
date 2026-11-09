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
//!   front-end false accepts below
//! ```
//!
//! Known front-end false accepts (oxc_regular_expression 0.155; node v26.13.0 and RX1 refuse
//! each, M7-P4 RX1-0 report), not counted as crashes:
//! - rx-dupnames: a duplicate group name that oxc does not see, for example
//!   `/(?<a>x|(?<a>y))/` (a group inside a group of the same name). RX1 says
//!   DuplicateCaptureGroupName.
//! - `\c` with no control letter inside a class in Unicode mode, for example `/[\c]/u`. RX1
//!   says InvalidUnicodeEscape at the `c`.
#![no_main]

use ep9_frontend::{CompileError, SourceOpts, Strictness};
use ep9_rx1::{Error, Flags, ParseError};

/// True for a known front-end false accept (the module doc), given RX1's error.
fn known_front_end_false_accept(units: &[u16], e: &ParseError) -> bool {
    match e.error {
        Error::DuplicateCaptureGroupName => true,
        Error::InvalidUnicodeEscape => {
            e.pos >= 1
                && units.get(e.pos) == Some(&u16::from(b'c'))
                && units[e.pos - 1] == u16::from(b'\\')
        }
        _ => false,
    }
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
    let oxc_valid =
        match ep9_frontend::check_early_errors(&source, SourceOpts::script(Strictness::Sloppy)) {
            Ok(()) => true,
            Err(CompileError::Syntax(_)) => false,
            Err(CompileError::Range(_)) => return,
        };
    let units: Vec<u16> = text.encode_utf16().collect();
    let rx1_valid = match ep9_rx1::parse(&units, flags) {
        Ok(_) => true,
        Err(e) if e.error.is_stack_overflow() => return,
        Err(e) if oxc_valid && known_front_end_false_accept(&units, &e) => return,
        Err(_) => false,
    };
    assert!(
        rx1_valid == oxc_valid,
        "RX1 and the front end disagree: rx1 valid {rx1_valid}, oxc valid {oxc_valid}"
    );
});
