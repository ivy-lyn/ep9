//! A `\u` escape with a `+` sign before its hex digits (route FUZZ-RX-M9P2; the regress fork
//! patch 0008, `third_party/patches/regress/`). regress read the digits with Rust's
//! `from_str_radix`, which takes a leading `+`, so `/\u+0ff/` was U+00FF. HexDigits have no
//! sign (ECMA-262 22.2.1): without `u` and `v`, `\u` is the Annex B identity escape `u` (then
//! `+` is a quantifier, or a class member); with `u` or `v`, and in a group name in every
//! mode, it is a SyntaxError. Each expected result is the result of `new RegExp(pattern,
//! flags)` and `test` or `exec` in node v26.13.1 (the oracle, as in `tests/node_pairs.rs`).
//! All patterns are public synthetic ones.

use ep9_regexp::{Backend, ExecResult, Flags, Input, Rx0, Rx0Scratch};

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn flags(letters: &str) -> Flags {
    Flags::parse(&units(letters)).expect("valid flag letters")
}

/// Whether `new RegExp(pattern, flags)` is a SyntaxError.
fn is_syntax_error(pattern: &str, letters: &str) -> bool {
    Rx0::compile(&units(pattern), flags(letters)).is_err()
}

/// The span of the first match of `pattern` in `subject` (`exec` from index 0), or `None`.
fn exec(pattern: &str, letters: &str, subject: &[u16]) -> Option<(usize, usize)> {
    let program = Rx0::compile(&units(pattern), flags(letters)).expect("a valid pattern");
    let mut caps = Vec::new();
    let mut scratch = Rx0Scratch::default();
    match Rx0::exec(
        &program,
        Input::Utf16(subject),
        0,
        &mut caps,
        &mut scratch,
        &mut || false,
    ) {
        ExecResult::Match => caps[0],
        ExecResult::NoMatch => None,
        r => panic!("{pattern}: {r:?}"),
    }
}

/// `new RegExp(pattern, flags).test(subject)`.
fn test(pattern: &str, letters: &str, subject: &str) -> bool {
    exec(pattern, letters, &units(subject)).is_some()
}

#[test]
fn annex_b_plus_after_u_is_identity_u() {
    // The class is `u`, `+`, `f`, `f`, `f-z` (regress read U+0FFF-z, out of order).
    assert!(!is_syntax_error(r"[\u+fff-z]", ""));
    for subject in ["u", "+", "f", "q", "z"] {
        assert!(test(r"[\u+fff-z]", "", subject), "{subject}");
    }
    assert!(!test(r"[\u+fff-z]", "", "{"));
    assert!(!test(r"[\u+fff-z]", "", "\u{fff}"));
    // The range `1-0` is out of order (regress took U+0001-0).
    assert!(is_syntax_error(r"[\u+001-0]", ""));
}

#[test]
fn plus_sign_escape_match_results_equal_node() {
    assert!(test(r"\u+0ff", "", "uu0ff"));
    assert!(!test(r"\u+0ff", "", "\u{ff}"));
    assert_eq!(exec(r"\u+0ff", "", &units("xuu0ff")), Some((1, 6)));
    assert!(test(r"[\u+041]", "", "+"));
    assert!(!test(r"[\u+041]", "", "A"));
    assert!(test(r"[\u+13D]", "", "+"));
    assert!(!test(r"[\u+13D]", "", "\u{13d}"));
    // A lead surrogate escape, then the identity escape `u`: U+D83D, `u+`, `DE00`.
    let mut subject = vec![0xD83D];
    subject.extend(units("uuDE00"));
    assert_eq!(exec(r"\uD83D\u+DE00", "", &subject), Some((0, 7)));
}

#[test]
fn unicode_mode_plus_sign_escapes_throw() {
    for (pattern, letters) in [
        (r"[\u+1F6]", "u"),
        (r"[\u+1F6]", "v"),
        (r"\u{+1F600}", "u"),
        (r"\u{+1F600}", "v"),
        (r"\u+0041", "u"),
        (r"\u+0041", "v"),
        (r"\uD83D\u+DE00", "u"),
        (r"\uD83D\u+DE00", "v"),
    ] {
        assert!(is_syntax_error(pattern, letters), "/{pattern}/{letters}");
    }
}

#[test]
fn group_name_plus_sign_escape_throws() {
    for letters in ["", "u", "v"] {
        assert!(is_syntax_error(r"(?<a\u+041>x)", letters), "/{letters}");
        assert!(is_syntax_error(r"(?<a\u{+41}>x)", letters), "/{letters}");
        assert!(
            is_syntax_error(r"(?<a>x)\k<a\u+041>", letters),
            "/{letters}"
        );
    }
}
