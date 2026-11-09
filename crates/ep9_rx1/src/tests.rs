//! Unit tests of the parser port: verdicts and error texts against node (V8), and tree shapes.

use super::*;

fn flags(s: &str) -> Flags {
    let units: Vec<u16> = s.encode_utf16().collect();
    Flags::parse(&units).expect("valid flags")
}

/// (pattern, flags, the V8 error text or `None` for a pattern that parses). The values come
/// from node v26.13.0 (`new RegExp(pattern, flags)`; the message after `/<source>/<flags>: `).
/// Two rows are spec rows where V8 follows ICU: see [`SPEC_NOT_V8`].
static NODE_CASES: &[(&str, &str, Option<&str>)] = &[
    ("", "", None),
    ("a", "", None),
    ("abc", "", None),
    ("a|b|", "", None),
    ("|", "", None),
    ("a*", "", None),
    ("a+?", "", None),
    ("a{2}", "", None),
    ("a{2,}", "", None),
    ("a{2,3}", "", None),
    ("a{3,2}", "", Some("numbers out of order in {} quantifier")),
    ("a{3,2}", "u", Some("numbers out of order in {} quantifier")),
    ("a{", "", None),
    ("a{", "u", Some("Incomplete quantifier")),
    ("a{1", "", None),
    ("a{1,", "u", Some("Incomplete quantifier")),
    ("{", "", None),
    ("{1}", "", Some("Nothing to repeat")),
    ("}", "", None),
    ("}", "u", Some("Lone quantifier brackets")),
    ("]", "", None),
    ("]", "u", Some("Lone quantifier brackets")),
    ("*", "", Some("Nothing to repeat")),
    ("a**", "", Some("Nothing to repeat")),
    ("a{99999999999}", "", None),
    // Above 2^53 - 1: no early error in ECMA-262 (oxc_regular_expression 0.157 has one).
    ("a{9007199254740992}", "u", None),
    ("a{99999999999999999999}", "", None),
    ("a{1,99999999999999999999}", "v", None),
    ("((>{366666666666666666}))", "mv", None),
    (
        "a{99999999999999999999,1}",
        "",
        Some("numbers out of order in {} quantifier"),
    ),
    ("(", "", Some("Unterminated group")),
    (")", "", Some("Unmatched ')'")),
    ("(a", "", Some("Unterminated group")),
    ("(?:a)", "", None),
    ("(?a)", "", Some("Invalid group")),
    ("(?=a)*", "", None),
    ("(?=a)*", "u", Some("Invalid quantifier")),
    ("(?!a)+", "", None),
    ("(?<=a)*", "", Some("Invalid quantifier")),
    ("(?<!a)", "", None),
    ("(?<=a)?", "u", Some("Invalid quantifier")),
    ("(?i:a)", "", None),
    ("(?i-m:a)", "", None),
    ("(?-i:a)", "", None),
    ("(?-:a)", "", Some("Invalid flag group")),
    ("(?ii:a)", "", Some("Repeated flag in flag group")),
    ("(?i-i:a)", "", Some("Repeated flag in flag group")),
    ("(?--i:a)", "", Some("Multiple dashes in flag group")),
    ("(?i=a)", "", Some("Invalid group")),
    ("(?i<a>b)", "", Some("Invalid group")),
    ("(?x:a)", "", Some("Invalid group")),
    ("(?i)", "", Some("Invalid group")),
    ("(?ims-:a)", "", None),
    ("(?<a>x)", "", None),
    ("(?<a>x)(?<a>y)", "", Some("Duplicate capture group name")),
    ("(?<a>x)|(?<a>y)", "", None),
    ("((?<a>x)|(?<a>y))", "", None),
    (
        "(?<a>x)|(?<a>y)(?<a>z)",
        "",
        Some("Duplicate capture group name"),
    ),
    ("(?:(?<a>x)|(?<a>y))\\k<a>", "", None),
    ("(?<a>(?<a>x))", "", Some("Duplicate capture group name")),
    ("(?<a>x|(?<a>y))", "", Some("Duplicate capture group name")),
    ("(?<1a>x)", "", Some("Invalid capture group name")),
    ("(?<a1>x)", "", None),
    ("(?<$_>x)", "", None),
    ("(?<a-b>x)", "", Some("Invalid capture group name")),
    ("(?<>x)", "", Some("Invalid capture group name")),
    ("(?<a", "", Some("Invalid capture group name")),
    ("(?<\\u0061>x)", "", None),
    ("(?<\\u{61}>x)", "", None),
    ("(?<\\u{61}>x)", "u", None),
    ("(?<\\uD835\\uDC9C>x)", "", None),
    ("(?<\\uD835>x)", "", Some("Invalid capture group name")),
    ("(?<\\x61>x)", "", Some("Invalid capture group name")),
    ("(?<\u{3c0}>x)", "", None),
    ("(?<a\\u200C>x)", "", None),
    ("\\k<a>(?<a>x)", "", None),
    (
        "\\k<b>(?<a>x)",
        "",
        Some("Invalid named capture referenced"),
    ),
    ("\\k", "", None),
    ("\\k", "u", Some("Invalid named reference")),
    ("\\k<a>", "", None),
    ("\\k<a", "u", Some("Invalid capture group name")),
    ("(?<a>x)\\k", "", Some("Invalid named reference")),
    ("(?<a>\\k<a>)", "", None),
    ("[\\k]", "", None),
    ("[\\k](?<a>x)", "", Some("Invalid escape")),
    ("\\1", "", None),
    ("\\1", "u", Some("Invalid escape")),
    ("(a)\\1", "", None),
    ("(a)\\2", "", None),
    ("(a)\\2", "u", Some("Invalid escape")),
    ("\\1(a)", "", None),
    ("(a\\1)", "", None),
    ("\\8", "", None),
    ("\\8", "u", Some("Invalid escape")),
    ("\\0", "u", None),
    ("\\00", "u", Some("Invalid decimal escape")),
    ("\\01", "", None),
    ("\\012", "", None),
    ("\\377", "", None),
    ("\\400", "", None),
    ("\\", "", Some("\\ at end of pattern")),
    ("a\\", "", Some("\\ at end of pattern")),
    ("\\c", "", None),
    ("\\cA", "", None),
    ("\\c1", "", None),
    ("\\c", "u", Some("Invalid Unicode escape")),
    ("[\\c1]", "", None),
    ("[\\c_]", "", None),
    ("[\\c]", "", None),
    ("\\x", "", None),
    ("\\x1", "u", Some("Invalid escape")),
    ("\\x41", "u", None),
    ("\\u", "", None),
    ("\\u", "u", Some("Invalid Unicode escape")),
    ("\\u{110000}", "u", Some("Invalid Unicode escape")),
    ("\\u{10FFFF}", "u", None),
    ("\\u{}", "u", Some("Invalid Unicode escape")),
    ("\\u{61", "u", Some("Invalid Unicode escape")),
    ("\\uD83D\\uDE00", "u", None),
    ("\\a", "", None),
    ("\\a", "u", Some("Invalid escape")),
    ("\\-", "u", Some("Invalid escape")),
    ("\\/", "u", None),
    ("\\q", "v", Some("Invalid escape")),
    // The buffer boundaries proposal (`\A`, `\z`, `\Z`) is not ECMA-262: identity escapes.
    ("\\A", "", None),
    ("\\z", "", None),
    ("\\Z", "", None),
    ("\\A", "u", Some("Invalid escape")),
    ("\\Z", "u", Some("Invalid escape")),
    ("\\z", "v", Some("Invalid escape")),
    ("a\\z", "u", Some("Invalid escape")),
    ("[\\z]", "u", Some("Invalid escape")),
    ("[a-z]", "", None),
    ("[z-a]", "", Some("Range out of order in character class")),
    ("[\\d-z]", "", None),
    ("[\\d-z]", "u", Some("Invalid character class")),
    ("[a-\\d]", "u", Some("Invalid character class")),
    ("[-a]", "", None),
    ("[a-]", "", None),
    ("[", "", Some("Unterminated character class")),
    ("[a", "", Some("Unterminated character class")),
    ("[]", "", None),
    ("[^]", "", None),
    ("[\\b]", "", None),
    ("[\\-]", "u", None),
    ("[\\B]", "", None),
    ("[\\B]", "u", Some("Invalid escape")),
    ("[\\u{1F600}-\\u{1F64F}]", "u", None),
    ("\\p{L}", "u", None),
    ("\\p{L}", "", None),
    ("\\p{Letter}", "u", None),
    ("\\p{gc=L}", "u", None),
    ("\\p{General_Category=Lu}", "u", None),
    ("\\p{sc=Greek}", "u", None),
    ("\\p{Script_Extensions=Latn}", "u", None),
    ("\\p{scx=Hrkt}", "u", Some("Invalid property name")),
    ("\\p{Lowercase}", "u", None),
    ("\\P{Any}", "u", None),
    ("\\p{ASCII}", "u", None),
    ("\\p{Assigned}", "u", None),
    ("\\p{lowercase}", "u", Some("Invalid property name")),
    ("\\p{L", "u", Some("Invalid property name")),
    ("\\p{}", "u", Some("Invalid property name")),
    ("\\p", "u", Some("Invalid property name")),
    ("\\p{gc}", "u", Some("Invalid property name")),
    ("\\p{ASCII=Y}", "u", Some("Invalid property name")),
    ("\\p{sc=}", "u", Some("Invalid property name")),
    (
        "[\\p{Foo}]",
        "u",
        Some("Invalid property name in character class"),
    ),
    ("\\p{RGI_Emoji}", "u", Some("Invalid property name")),
    ("\\p{RGI_Emoji}", "v", None),
    ("\\P{RGI_Emoji}", "v", Some("Invalid property name")),
    (
        "[^\\p{RGI_Emoji}]",
        "v",
        Some("Negated character class may contain strings"),
    ),
    ("\\p{WSpace}", "u", None),
    ("[a&&b]", "v", None),
    ("[a--b]", "v", None),
    ("[a&&&b]", "v", Some("Invalid character in character class")),
    (
        "[a&&b--c]",
        "v",
        Some("Invalid set operation in character class"),
    ),
    (
        "[a--b&&c]",
        "v",
        Some("Invalid set operation in character class"),
    ),
    (
        "[ab--c]",
        "v",
        Some("Invalid set operation in character class"),
    ),
    (
        "[a-z--c]",
        "v",
        Some("Invalid set operation in character class"),
    ),
    ("[[a]--[b]]", "v", None),
    ("[a-[b]]", "v", Some("Invalid character class")),
    ("[a(]", "v", Some("Invalid character in character class")),
    ("[a&&]", "v", Some("Invalid character in character class")),
    (
        "[&&a]",
        "v",
        Some("Invalid set operation in character class"),
    ),
    (
        "[a!!b]",
        "v",
        Some("Invalid set operation in character class"),
    ),
    ("[a!b]", "v", None),
    ("[\\q{abc|d}]", "v", None),
    (
        "[^\\q{abc}]",
        "v",
        Some("Negated character class may contain strings"),
    ),
    ("[^\\q{a|b}]", "v", None),
    (
        "[^\\q{}]",
        "v",
        Some("Negated character class may contain strings"),
    ),
    ("[\\q{a]", "v", Some("Invalid character in character class")),
    ("[\\q]", "v", Some("Invalid escape")),
    ("[[[[a]]]]", "v", None),
    (
        "[^[\\q{ab}]]",
        "v",
        Some("Negated character class may contain strings"),
    ),
    ("[\\p{RGI_Emoji}&&a]", "v", None),
    ("[^\\p{RGI_Emoji}&&a]", "v", None),
    (
        "[^\\p{RGI_Emoji}--a]",
        "v",
        Some("Negated character class may contain strings"),
    ),
    ("[\\&]", "v", None),
    ("[\\!]", "v", None),
    ("\\&", "v", Some("Invalid escape")),
    ("[]", "v", None),
    ("[^]", "v", None),
    ("[a-]", "v", Some("Invalid character in character class")),
    ("[-a]", "v", Some("Invalid character in character class")),
    ("[a", "v", Some("Unterminated character class")),
    ("[a&&b", "v", Some("Unterminated character class")),
    ("[\\d-a]", "v", Some("Invalid character class")),
    ("[a-\\d]", "v", Some("Invalid character class")),
    ("[\\q{a}-b]", "v", Some("Invalid character class")),
    ("[a--]", "v", Some("Invalid character in character class")),
    ("^*", "", Some("Nothing to repeat")),
    ("$+", "", Some("Nothing to repeat")),
    ("\\b*", "", Some("Nothing to repeat")),
    ("a|*", "", Some("Nothing to repeat")),
    ("(*)", "", Some("Nothing to repeat")),
    ("(?:)*", "", None),
    ("()*", "", None),
    ("(?=)*", "", None),
    ("\\u{1F600}*", "u", None),
    ("\\uD83D", "u", None),
    ("[\\uD83D]", "u", None),
    ("x{2,1}?", "", Some("numbers out of order in {} quantifier")),
    ("x{1}{2}", "", Some("Nothing to repeat")),
    ("a??", "", None),
    ("a{,5}", "", None),
    ("a{,5}", "u", Some("Incomplete quantifier")),
    // An escaped `>` in a group name (spec rows, M7-P4 fixer r2 reviewer R6).
    ("(?<a\\u003e>x)", "", None),
    ("(?<a\\u003e>x)", "u", None),
    ("(?<a\\u{3e}>x)", "u", None),
    ("(?<a\\u003e>x)\\k<a>", "", None),
    ("(?<a\\u003e>x)\\k<a>", "u", None),
    ("(?<a\\u003e>x)\\k<a>", "v", None),
    ("(?<\\u003e>x)", "", Some("Invalid capture group name")),
    // Quantifier bounds above the cap (spec rows, M7-P4 fixer r2 reviewer R7).
    ("a{2147483649,2147483648}", "", None),
    ("a{2147483649,2147483648}", "u", None),
    ("a{2147483649,2147483648}", "v", None),
    ("a{99999999999,9999999999}", "", None),
    ("a{2147483648,2147483649}", "", None),
    (
        "a{00000000000000000003,2}",
        "",
        Some("numbers out of order in {} quantifier"),
    ),
    ("a{2,00000000000000000003}", "", None),
];

/// Rows of [`NODE_CASES`] where RX1 follows ECMA-262 and V8 does not (crate doc, "Known
/// gaps"): node v26.13.0 refuses `\p{scx=Hrkt}` (an empty ICU set) and takes `\p{WSpace}`
/// (an ICU alias that table 68 does not list); node takes a `\u` escape of `>` as the end of
/// a group name (ECMA-262 22.2.1: GroupName ends at a source `>`; 22.2.1.1: an escaped code
/// point of a RegExpIdentifierPart must be an IdentifierPartChar, and U+003E is not); node
/// compares the bounds of `{n,m}` after it caps them at 2^31 - 1 (22.2.1.1: the MV of the
/// first DecimalDigits must not be larger than the MV of the second). oxc (the front end)
/// refuses the escaped `>` and the bounds out of order too.
static SPEC_NOT_V8: &[(&str, &str, Option<&str>)] = &[
    ("\\p{scx=Hrkt}", "u", None),
    ("\\p{WSpace}", "u", Some("Invalid property name")),
    ("(?<a\\u003e>x)", "", Some("Invalid capture group name")),
    ("(?<a\\u003e>x)", "u", Some("Invalid capture group name")),
    ("(?<a\\u{3e}>x)", "u", Some("Invalid capture group name")),
    (
        "(?<a\\u003e>x)\\k<a>",
        "",
        Some("Invalid capture group name"),
    ),
    (
        "(?<a\\u003e>x)\\k<a>",
        "u",
        Some("Invalid capture group name"),
    ),
    (
        "(?<a\\u003e>x)\\k<a>",
        "v",
        Some("Invalid capture group name"),
    ),
    (
        "a{2147483649,2147483648}",
        "",
        Some("numbers out of order in {} quantifier"),
    ),
    (
        "a{2147483649,2147483648}",
        "u",
        Some("numbers out of order in {} quantifier"),
    ),
    (
        "a{2147483649,2147483648}",
        "v",
        Some("numbers out of order in {} quantifier"),
    ),
    (
        "a{99999999999,9999999999}",
        "",
        Some("numbers out of order in {} quantifier"),
    ),
];

#[test]
fn verdicts_and_error_texts_match_node() {
    let mut bad = Vec::new();
    for &(pattern, f, node) in NODE_CASES {
        let want = SPEC_NOT_V8
            .iter()
            .find(|r| r.0 == pattern && r.1 == f)
            .map_or(node, |r| r.2);
        let got = parse_str(pattern, flags(f)).err().map(|e| e.error.text());
        if got != want {
            bad.push(format!("/{pattern}/{f}: rx1 {got:?}, want {want:?}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} differ:\n{}",
        bad.len(),
        NODE_CASES.len(),
        bad.join("\n")
    );
}

#[test]
fn spec_rows_differ_from_node() {
    // Each SPEC_NOT_V8 row is a NODE_CASES row with the other verdict, so the list stays true.
    for r in SPEC_NOT_V8 {
        let node = NODE_CASES
            .iter()
            .find(|c| c.0 == r.0 && c.1 == r.1)
            .expect("a node row");
        assert_ne!(node.2.is_some(), r.2.is_some(), "{r:?}");
    }
}

fn tree(pattern: &str, f: &str) -> Tree {
    parse_str(pattern, flags(f)).expect("parses").tree
}

#[test]
fn text_and_quantifier_shapes() {
    // "abc*": the atom "ab" then c* (V8 PopLastAtom splits the last character off).
    let t = tree("abc*", "");
    let Tree::Alternative(terms) = t else {
        panic!("{t:?}")
    };
    assert_eq!(terms[0], Tree::Atom(vec![0x61, 0x62]));
    let Tree::Quantifier(q) = &terms[1] else {
        panic!()
    };
    assert_eq!(
        (q.min, q.max, q.kind),
        (0, INFINITY, QuantifierType::Greedy)
    );
    assert_eq!(*q.body, Tree::Atom(vec![0x63]));
    // An atom and a class join into one Text.
    assert!(matches!(tree("a[b]", ""), Tree::Text(e) if e.len() == 2));
    // A quantifier on an empty group is dropped when its minimum is 0 (V8 max_match == 0).
    assert_eq!(tree("(?:)*", ""), Tree::Empty);
    assert!(matches!(tree("(?:)+", ""), Tree::Group(_)));
    // Lookahead quantifiers without u (Annex B) wrap the lookaround.
    assert!(matches!(tree("(?=a)", ""), Tree::Lookaround(_)));
    // A big count saturates at INFINITY.
    let Tree::Quantifier(q) = tree("a{99999999999,}", "") else {
        panic!()
    };
    assert_eq!((q.min, q.max), (INFINITY, INFINITY));
}

#[test]
fn unicode_text_shapes() {
    // A supplementary character is a 2-unit atom in Unicode mode, 2 characters without it.
    assert_eq!(tree("\u{1F600}", "u"), Tree::Atom(vec![0xd83d, 0xde00]));
    assert_eq!(tree("\\u{1F600}", "u"), Tree::Atom(vec![0xd83d, 0xde00]));
    assert_eq!(
        tree("\\uD83D\\uDE00", "u"),
        Tree::Atom(vec![0xd83d, 0xde00])
    );
    // Without u, \u{1F600} is u repeated 1F600 times? No: {1F600} is not a quantifier, so
    // the text is "u{1F600}".
    assert_eq!(
        tree("\\u{1F600}", ""),
        Tree::Atom("u{1F600}".encode_utf16().collect())
    );
    // A lone surrogate in Unicode mode is its own class term.
    let t = tree("a\\uD83D", "u");
    let Tree::Alternative(terms) = t else {
        panic!("{t:?}")
    };
    assert_eq!(
        terms[1],
        Tree::ClassRanges(ClassRanges {
            items: vec![ClassItem::Range(0xd83d, 0xd83d)],
            negated: false
        })
    );
    // A quantifier after a surrogate pair takes the whole pair in Unicode mode only.
    let Tree::Quantifier(q) = tree("\u{1F600}+", "u") else {
        panic!()
    };
    assert_eq!(*q.body, Tree::Atom(vec![0xd83d, 0xde00]));
    let Tree::Alternative(t) = tree("\u{1F600}+", "") else {
        panic!()
    };
    assert_eq!(t[0], Tree::Atom(vec![0xd83d]));
}

#[test]
fn captures_and_names() {
    let p = parse_str("(?<a>x)|(?<a>y)(z)\\k<a>", Flags::empty()).unwrap();
    assert_eq!(p.capture_count, 3);
    let a: Vec<u16> = "a".encode_utf16().collect();
    assert_eq!(p.named_captures, vec![(a.clone(), 1), (a.clone(), 2)]);
    // The named back reference gets both groups.
    let Tree::Disjunction(alts) = &p.tree else {
        panic!()
    };
    let Tree::Alternative(terms) = &alts[1] else {
        panic!()
    };
    assert_eq!(
        terms[2],
        Tree::BackReference(BackReference {
            captures: vec![1, 2],
            name: Some(a)
        })
    );
    // A forward numbered reference is a back reference (the scan counts the groups).
    let p = parse_str("\\2(a)(b)", Flags::empty()).unwrap();
    let Tree::Alternative(terms) = &p.tree else {
        panic!()
    };
    assert!(matches!(&terms[0], Tree::BackReference(r) if r.captures == [2]));
    // \2 with one group is the octal escape \2 without u.
    let Tree::Alternative(terms) = tree("(a)\\2", "") else {
        panic!()
    };
    assert_eq!(terms[1], Tree::Atom(vec![2]));
    // A name with a surrogate pair escape is one code point (two units).
    let p = parse_str("(?<\\uD835\\uDC9C>x)", Flags::empty()).unwrap();
    assert_eq!(p.named_captures[0].0, vec![0xd835, 0xdc9c]);
}

#[test]
fn modifiers_set_the_group_flags() {
    let Tree::Group(g) = tree("(?i-m:a)", "m") else {
        panic!()
    };
    assert_eq!(g.flags, Flags::IGNORE_CASE);
    // ^ inside (?m:) is a line start.
    let Tree::Group(g) = tree("(?m:^)", "") else {
        panic!()
    };
    assert_eq!(*g.body, Tree::Assertion(AssertionType::StartOfLine));
}

#[test]
fn classes_keep_escapes_and_properties_by_name() {
    let Tree::ClassRanges(c) = tree("[^a-c\\d\\p{Lu}]", "u") else {
        panic!()
    };
    assert!(c.negated);
    assert_eq!(
        c.items,
        vec![
            ClassItem::Range(0x61, 0x63),
            ClassItem::Standard(StandardSet::Digit),
            ClassItem::Property {
                property: Property::GeneralCategory("Lu"),
                negated: false
            },
        ]
    );
    let Tree::ClassRanges(c) = tree("\\P{sc=Greek}", "u") else {
        panic!()
    };
    assert_eq!(
        c.items,
        vec![ClassItem::Property {
            property: Property::Script("Grek"),
            negated: true
        }]
    );
    // Without u, \p is the identity escape p.
    assert_eq!(
        tree("\\p{L}", ""),
        Tree::Atom("p{L}".encode_utf16().collect())
    );
}

#[test]
fn v_mode_set_expressions() {
    let Tree::ClassSetExpression(e) = tree("[\\p{L}--[a-z]]", "v") else {
        panic!()
    };
    assert_eq!(e.operation, ClassSetOperation::Subtraction);
    assert_eq!(e.operands.len(), 2);
    assert!(matches!(&e.operands[1], Tree::ClassSetExpression(_)));
    let Tree::ClassSetExpression(e) = tree("[\\q{abc|d|}a]", "v") else {
        panic!()
    };
    assert!(e.may_contain_strings);
    let Tree::ClassSetOperand(op) = &e.operands[0] else {
        panic!()
    };
    // "d" and "a" are code points; "abc" and "" are strings, longest first.
    assert_eq!(
        op.items,
        vec![ClassItem::Range(0x64, 0x64), ClassItem::Range(0x61, 0x61)]
    );
    let keys: Vec<&[u32]> = op.strings.keys().map(|k| k.0.as_slice()).collect();
    assert_eq!(keys, vec![&[0x61, 0x62, 0x63][..], &[][..]]);
    let Tree::ClassSetExpression(e) = tree("[\\p{RGI_Emoji}&&\\p{Emoji}]", "v") else {
        panic!()
    };
    assert_eq!(e.operation, ClassSetOperation::Intersection);
    assert!(!e.may_contain_strings);
}

#[test]
fn latin1_input_never_pairs_surrogates() {
    // One byte per unit: the same verdicts as the UTF-16 form for Latin-1 text.
    for (p, f) in [
        ("caf\u{e9}+", ""),
        ("[\u{e0}-\u{ff}]", "u"),
        ("(?<\u{e9}>x)", ""),
    ] {
        let bytes: Vec<u8> = p.chars().map(|c| c as u32 as u8).collect();
        assert_eq!(
            parse_latin1(&bytes, flags(f)),
            parse_str(p, flags(f)),
            "{p}"
        );
    }
}

#[test]
fn error_positions_are_v8s() {
    let e = parse_str("ab)", Flags::empty()).unwrap_err();
    assert_eq!((e.error, e.pos), (Error::UnmatchedParen, 2));
    let e = parse_str("a{2,1}", Flags::empty()).unwrap_err();
    assert_eq!(e.error, Error::RangeOutOfOrder);
    let e = parse_str("(", Flags::empty()).unwrap_err();
    assert_eq!((e.error, e.pos), (Error::UnterminatedGroup, 1));
}

/// The fuzz smoke input of CI run 42552778227 (minimized): flags byte 0xc7 (`dgivy`), then
/// `\z`. oxc_regular_expression takes `\z` in Unicode mode as a buffer boundary assertion (a
/// TC39 proposal). ECMA-262 22.2.1 has no such Assertion, and IdentityEscape[+UnicodeMode] is
/// only SyntaxCharacter or `/`, so `/\z/v` is a SyntaxError (node v26.13.0: "Invalid escape").
/// The regexp fuzz target skips this difference by the error kind and the position of the
/// letter, so this test keeps both.
#[test]
fn buffer_boundary_escapes_are_errors_in_unicode_mode() {
    let fuzz_flags = Flags::from_bits(0xc7);
    assert_eq!(fuzz_flags.to_letters(), "dgivy");
    let e = parse_str("\\z", fuzz_flags).unwrap_err();
    assert_eq!((e.error, e.pos), (Error::InvalidEscape, 1));
    for f in ["u", "v", "iu", "dgivy"] {
        for (pattern, pos) in [
            ("\\A", 1),
            ("\\z", 1),
            ("\\Z", 1),
            ("a\\z", 2),
            ("(?:^\\A)", 5),
        ] {
            let e = parse_str(pattern, flags(f)).unwrap_err();
            assert_eq!(
                (e.error, e.pos),
                (Error::InvalidEscape, pos),
                "/{pattern}/{f}"
            );
        }
    }
    for pattern in ["\\A", "\\z", "\\Z", "^\\Aa\\z\\Z$"] {
        assert!(parse_str(pattern, Flags::empty()).is_ok(), "/{pattern}/");
    }
}

#[test]
fn too_many_captures() {
    let max = "()".repeat(MAX_CAPTURES as usize);
    assert!(parse_str(&max, Flags::empty()).is_ok());
    let over = format!("{max}()");
    assert_eq!(
        parse_str(&over, Flags::empty()).unwrap_err().error,
        Error::TooManyCaptures
    );
}

/// Alternatives that repeat a group name and add another capture each (`(?<a>x)()|...`): the
/// non-participating intervals of each alternative merge when they overlap or touch, so the
/// list stays disjoint (a debug assert on the push) and short, and the duplicate-name scan
/// costs O(N^2) interval steps for N alternatives, not O(N^3) (M7-P4 fixer r2, Codex r1
/// rx1-0:dupname-cubic: N = 8,000 took 24.4 s in release). A count, not a time.
#[test]
fn repeated_names_across_alternatives_merge_their_intervals() {
    let three = "(?<a>x)()|(?<a>x)()|(?<a>x)()";
    let p = parse_str(three, Flags::empty()).expect("parses");
    assert_eq!(p.capture_count, 6);
    for n in [1_000u64, 8_000] {
        let pattern = vec!["(?<a>x)()"; n as usize].join("|");
        let before = parser::INTERVAL_STEPS.with(core::cell::Cell::get);
        let p = parse_str(&pattern, Flags::empty()).expect("parses");
        let steps = parser::INTERVAL_STEPS.with(core::cell::Cell::get) - before;
        assert_eq!(u64::from(p.capture_count), 2 * n);
        // About n^2 / 2 with merged intervals; about n^3 / 6 without.
        assert!(steps <= n * n, "{n} alternatives: {steps} interval steps");
    }
}

/// The nesting cap: at the cap the parse and the drop of the tree fit in a 1 MiB thread stack
/// (the Windows main thread size), and one more level is a StackOverflow error, not a crash.
#[test]
fn nesting_cap_is_an_error_not_a_crash() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(nesting_cap_body)
        .unwrap()
        .join()
        .unwrap();
}

fn nesting_cap_body() {
    let depth = MAX_NESTING - 1;
    let groups = format!("{}a{}", "(".repeat(depth), ")".repeat(depth));
    let p = parse_str(&groups, Flags::empty()).unwrap();
    assert_eq!(p.capture_count as usize, depth);
    drop(p);
    let classes = format!("{}a{}", "[".repeat(depth), "]".repeat(depth));
    assert!(parse_str(&classes, flags("v")).is_ok());
    for deep in [
        format!(
            "{}a{}",
            "(".repeat(MAX_NESTING + 1),
            ")".repeat(MAX_NESTING + 1)
        ),
        format!(
            "{}a{}",
            "[".repeat(MAX_NESTING + 1),
            "]".repeat(MAX_NESTING + 1)
        ),
        format!(
            "{}[[a]]{}",
            "(?:".repeat(MAX_NESTING - 1),
            ")".repeat(MAX_NESTING - 1)
        ),
    ] {
        let e = parse_str(&deep, flags("v")).unwrap_err();
        assert_eq!(e.error, Error::StackOverflow);
        assert!(e.error.is_stack_overflow());
    }
}

#[test]
fn the_ucd_version_is_17() {
    assert_eq!(UCD_VERSION, "17.0.0");
}
