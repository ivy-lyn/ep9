//! A `\u` escape with a `+` sign before its hex digits (route FUZZ-RX-M9P2; the regress fork
//! patch 0008, `third_party/patches/regress/`). regress read the digits with Rust's
//! `from_str_radix`, which takes a leading `+`, so `/\u+0ff/` was U+00FF. HexDigits have no
//! sign (ECMA-262 22.2.1): without `u` and `v`, `\u` is the Annex B identity escape `u` (then
//! `+` is a quantifier, or a class member); with `u` or `v`, and in a group name in every
//! mode, it is a SyntaxError. Each expected result is the result of `new RegExp(pattern,
//! flags)` and `test` or `exec` in node v26.13.1 (the oracle, as in `tests/node_pairs.rs`).
//! All patterns are public synthetic ones.
//!
//! Rx0 rewrites some `\u` forms before regress reads the pattern (`legacy_chars` and
//! `unicode_chars` in `src/lib.rs`), so a lead surrogate escape goes around the parser's
//! look-ahead for a trail. The `regress_parser_*` tests give the pattern to regress directly
//! (the `utf16` feature of this crate's regress dependency), with no rewrite: they fail if
//! the parser goes back to after the second `\u` when the trail is not a trail surrogate
//! escape (patch 0008 moves that save point to before the `\u`).

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
    }
}

/// The two groups `a` and `aA`: a signed escape in `\k<...>` must be a SyntaxError, not a
/// reference to `aA` (the old parser read `\u+041` and `\u{+41}` as `A`; it read the 4 units
/// `+004` of `\u+0041` as U+0004, not a name character, so that form fails in both). The same
/// reference with an unsigned escape is `aA` and matches.
const SIGNED_REFS: [&str; 3] = [
    r"(?<a>x)(?<aA>y)\k<a\u+041>",
    r"(?<a>x)(?<aA>y)\k<a\u+0041>",
    r"(?<a>x)(?<aA>y)\k<a\u{+41}>",
];
const UNSIGNED_REFS: [&str; 2] = [
    r"^(?<a>x)(?<aA>y)\k<a\u0041>$",
    r"^(?<a>x)(?<aA>y)\k<a\u{41}>$",
];

#[test]
fn named_reference_plus_sign_escape_throws() {
    for letters in ["", "u", "v"] {
        for pattern in SIGNED_REFS {
            assert!(is_syntax_error(pattern, letters), "/{pattern}/{letters}");
        }
        for pattern in UNSIGNED_REFS {
            assert!(test(pattern, letters, "xyy"), "/{pattern}/{letters}");
            assert!(!test(pattern, letters, "xyx"), "/{pattern}/{letters}");
        }
    }
}

/// regress flags as `Rx0::compile` sets them (`v` sets `unicode` too).
fn regress_flags(letters: &str) -> regress::Flags {
    regress::Flags {
        unicode: letters.contains('u') || letters.contains('v'),
        unicode_sets: letters.contains('v'),
        ..regress::Flags::default()
    }
}

/// Whether regress, with no Rx0 rewrite, refuses `pattern`.
fn regress_is_syntax_error(pattern: &str, letters: &str) -> bool {
    regress::Regex::with_flags(pattern, regress_flags(letters)).is_err()
}

/// regress, with no Rx0 rewrite: the span of the first match in `subject` (UTF-16 in Unicode
/// mode, UCS-2 otherwise, as Rx0 reads `Input::Utf16`).
fn regress_exec(pattern: &str, letters: &str, subject: &[u16]) -> Option<(usize, usize)> {
    let flags = regress_flags(letters);
    let re = regress::Regex::with_flags(pattern, flags).expect("a valid pattern");
    let m = if flags.unicode {
        re.find_from_utf16(subject, 0).next()
    } else {
        re.find_from_ucs2(subject, 0).next()
    };
    m.map(|m| (m.start(), m.end()))
}

/// The lone lead surrogate U+D83D, then `rest`.
fn lead(rest: &str) -> Vec<u16> {
    let mut v = vec![0xD83D];
    v.extend(rest.encode_utf16());
    v
}

#[test]
fn regress_parser_lead_escape_keeps_the_next_escape() {
    // Without `u` and `v`: `\u0041` after a lead surrogate escape is the next escape, `A`; a
    // signed `\u+DE00` there is the identity escape `u`, then `+`, then `DE00`.
    assert_eq!(regress_exec(r"\uD83D\u0041", "", &lead("A")), Some((0, 2)));
    assert_eq!(regress_exec(r"^\uD83D\u0041$", "", &lead("0041")), None);
    assert_eq!(
        regress_exec(r"[\uD83D\u0041]", "", &units("A")),
        Some((0, 1))
    );
    assert_eq!(regress_exec(r"[\uD83D\u0041]", "", &units("0")), None);
    assert_eq!(
        regress_exec(r"\uD83D\u+DE00", "", &lead("uuDE00")),
        Some((0, 7))
    );
    assert_eq!(regress_exec(r"^\uD83D\u+DE00$", "", &lead("DE00")), None);
    assert_eq!(
        regress_exec(r"\uD83D\uzzzz", "", &lead("uzzzz")),
        Some((0, 6))
    );
    // With `u` or `v`: the lone lead surrogate, then `A`.
    for letters in ["u", "v"] {
        assert_eq!(
            regress_exec(r"\uD83D\u0041", letters, &lead("A")),
            Some((0, 2)),
            "/{letters}"
        );
        assert_eq!(
            regress_exec(r"^\uD83D\u0041$", letters, &lead("0041")),
            None,
            "/{letters}"
        );
        assert_eq!(
            regress_exec(r"[\uD83D\u0041]", letters, &units("A")),
            Some((0, 1)),
            "/{letters}"
        );
    }
}

#[test]
fn regress_parser_lead_escape_then_bad_escape_throws_in_unicode_mode() {
    for letters in ["u", "v"] {
        for pattern in [r"\uD83D\u+DE00", r"[\uD83D\u+DE00]", r"\uD83D\uzzzz"] {
            assert!(
                regress_is_syntax_error(pattern, letters),
                "/{pattern}/{letters}"
            );
        }
    }
}

#[test]
fn regress_parser_named_reference_plus_sign_escape_throws() {
    for letters in ["", "u", "v"] {
        for pattern in SIGNED_REFS {
            assert!(
                regress_is_syntax_error(pattern, letters),
                "/{pattern}/{letters}"
            );
        }
        for pattern in UNSIGNED_REFS {
            assert_eq!(
                regress_exec(pattern, letters, &units("xyy")),
                Some((0, 3)),
                "/{pattern}/{letters}"
            );
            assert_eq!(
                regress_exec(pattern, letters, &units("xyx")),
                None,
                "/{pattern}/{letters}"
            );
        }
    }
}

/// regress through its public flag strings (`regress::Flags::from`): `"v"` sets only
/// `unicode_sets`, not `unicode` as `Rx0::compile` does. `v` is Unicode mode all the same, so a
/// `\u` that is not a RegExpUnicodeEscapeSequence is a SyntaxError (node: the same).
#[test]
fn regress_flag_string_plus_sign_escape_throws() {
    for letters in ["u", "v"] {
        for pattern in [
            r"[\u+fff-z]",
            r"\u+0ff",
            r"\u{+1F600}",
            r"\uD83D\u+DE00",
            r"[\uD83D\u+DE00]",
        ] {
            assert!(
                regress::Regex::with_flags(pattern, letters).is_err(),
                "/{pattern}/{letters}"
            );
        }
    }
}
