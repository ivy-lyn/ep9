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
//!        valid != RX1 valid -> panic, except the known front-end difference below (front
//!        end valid, RX1 InvalidCaptureGroupName at a group name that has a listed code
//!        point; a group-name code point that the two take differently outside the list is a
//!        crash)
//!   2. RX0: ep9_regexp::Rx0::compile(text, flags)
//!        valid != RX1 valid -> panic, except the known RX0 differences below
//!        both valid: exec on the pattern text as the subject, at most POLLS polls;
//!        a match: one span per group (RX1 capture_count + 1), each inside the subject
//!   3. the pattern is one class (Tree::ClassRanges, ClassSetOperand, ClassSetExpression)
//!        RX1 set (class_ranges / class_set; built also when a known difference skips the
//!        compare) against RX0 "^(?:<text>)$" (flags i s u v) on
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
//! - Validity: a `v` class range that ends in `\b`, for example `/[$-\b]/v`. RX1 says
//!   OutOfOrderCharacterClass (`\b` is U+0008 in a class, ECMA-262 22.2.1), and node and ep9's
//!   front end refuse it; regress reads that `\b` as `b` (U+0062; the set row of `[\b]`
//!   below), so it takes the range when its start is U+0062 or lower. The target skips an RX1
//!   OutOfOrderCharacterClass with `v` only when the error position is just after such a `-\b`
//!   (the `\` opens an escape) and RX0 refuses the text with that `\b` (and a `\b` range
//!   start) written `\x08` (found by the M7-P5 fixer r7 10-minute run).
//! - The proof of the 3 rows above (fixer r2 and r7, node v26.13.0; in ep9 `new RegExp` is RX0
//!   and the literal is the front end; "Syntax" = a SyntaxError):
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
//!   /[$-\b]/v        Syntax, Syntax          ok, Syntax             OutOfOrderCharacterClass, 5
//!   /[a-\b]/v        Syntax, Syntax          ok, Syntax             OutOfOrderCharacterClass, 5
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
//! - Sets: a single `&` (not `&&`, not escaped) right after the first ClassSetOperand of a
//!   `v` class (the outer class or a nested one), for example `/[a&b]/v` and `/[\!&]/v`.
//!   regress drops that operand (a character, an escape, a nested class, `\q{..}`, `\w`,
//!   `\p{..}`), so it does not match `a` and `!` there (node and RX1 do: a single `&` is a
//!   ClassSetCharacter, ECMA-262 22.2.1; found by the M7-P5 fixer r1 and r2 runs,
//!   `/[\!&function \#&]/v`, `/[L&Po\!\#a]/v`). regress keeps a range before the `&`, a later
//!   operand (`/[x&y&z]/v` drops only `x`) and a `&` at the class start or inside `\q{..}`; the
//!   target compares those sets (fixer r7). Proof (fixer r2 and r7, node v26.13.0; ep9
//!   `new RegExp` = RX0; each on `^(?:<class>)$`):
//!
//! ```text
//!   class            subject      node    ep9
//!   /[a&b]/v         "a"          true    false
//!   /[L&Po\!\#a]/v   "L"          true    false
//!   /[\!&]/v         "!"          true    false
//!   /[[a]&b]/v       "a"          true    false
//!   /[\q{a}&b]/v     "a"          true    false
//!   /[\w&]/v         "a"          true    false
//!   /[^a&]/v         "a"          false   true
//!   /[x[a&b]]/v      "a"          true    false
//!   /[\u{1F600}&]/v  "\u{1F600}"  true    false
//!   /[x&y&z]/v       "x"          true    false
//!   /[x&y&z]/v       "y"          true    true
//!   /[a-c&]/v        "a"          true    true
//!   /[&a-c]/v        "a"          true    true
//!   /[\q{a&b}x]/v    "x"          true    true
//!   /[[a]--&]/v      "&"          false   false
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
//!
//! Known front-end difference (check 1; DL M9-PRE-FUZZRX-FORM, route FUZZ-RX-M8P5):
//! - Validity: a code point that the front end and RX1 take differently in a group name, raw,
//!   as `\u{..}`, as a `\uHHHH` escape or as a `\uHHHH\uHHHH` surrogate-pair escape, for
//!   example `/(?<\u{3D000}>a)/u` (the seed `named-groups-ucd18-u`) and `/(?<\u0558>a)/u`. The
//!   front end (oxc_regular_expression with `unicode-id-start` 1.5.0, Unicode 18.0.0) and RX1
//!   (UCD 17, DL M9-PRE-UNICODE) have different ID_Start and ID_Continue tables: at the M9-P1
//!   merge, 12,860 code points in 38 ranges are ID_Continue for the front end and not for
//!   RX1 (12,823 in 27 ranges also ID_Start; U+3D000..U+3FC3F is 11,328 of them), and 0 the
//!   other way. These 38 ranges are a reviewed list (`KNOWN_ID_DIFFERENCE`). Check 1 compares
//!   validity first. It accepts a difference only when the front end takes the text, RX1
//!   says InvalidCaptureGroupName, the RX1 error position (a UTF-16 unit index) is in a group
//!   name of the text (`(?<..>`, and `\k<..>` with `u`, `v` or a named group; from the unit
//!   after `<` to the `>` or the text end), and that name has a code point that the two
//!   parsers take differently (`id_differs`; the name is decoded: raw, `\u{..}` with hex
//!   digits only, so `\u{+558}` is no code point, `\uHHHH` and a `\uHHHH\uHHHH` surrogate
//!   pair). Such a code point in a literal or elsewhere outside a group name gives no skip
//!   (M9-P1 Codex r2 F2), and an RX1 InvalidCaptureGroupName at another name gives no skip
//!   either (M9-P1 fixer r5). A group-name code point that they take differently and that is
//!   not in the list is a crash ("new front-end/RX1 identifier difference at U+XXXX"): a new
//!   find, for example after an oxc or RX1 update. Check 2 does not compare an RX0 refusal
//!   with `v` (the RX0 row above); this skip needs an RX1 refusal, so check 1 still compares
//!   every text that RX1 takes with `v`. At start (`fuzz_target!` init), the target probes a
//!   sample of the list (the first, middle and last code point of each range must differ, the
//!   code point before and after each range must not), so a stale list stops the run. The
//!   move of RX1 to Unicode 18 is in the M2-exit G8 batch; that change must make the list
//!   again (the start check stops the run until it does).
#![no_main]

use std::cell::RefCell;

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

/// The reviewed front-end/RX1 identifier difference of check 1 (the module doc): the
/// code points, as inclusive ranges in order, that are ID_Continue for the front end
/// (oxc_regular_expression 0.158.0 with `unicode-id-start` 1.5.0, Unicode 18.0.0) and not for
/// RX1 (`ep9_rx1` unicode_tables.rs, UCD 17.0.0): 12,860 code points in 38 ranges (12,823 of
/// them in 27 ranges are also ID_Start; the other way, 0). Made at the M9-P1 merge from a
/// probe of every code point U+0080..U+10FFFF with a copy of [`probe_id_differs`],
/// against the real `ep9_frontend` and `ep9_rx1`: the probe set is equal to the difference of
/// the two tables. Make it again after an oxc or RX1 Unicode update.
const KNOWN_ID_DIFFERENCE: [(u32, u32); 38] = [
    (0x558, 0x558),
    (0x58b, 0x58c),
    (0x5c8, 0x5c9),
    (0xb53, 0xb54),
    (0x1ade, 0x1adf),
    (0x1aec, 0x1af0),
    (0x208f, 0x208f),
    (0x209d, 0x209f),
    (0xa7dd, 0xa7dd),
    (0xa7e2, 0xa7e2),
    (0xab6c, 0xab6d),
    (0x107bb, 0x107bf),
    (0x10ecb, 0x10ecf),
    (0x10ed9, 0x10eee),
    (0x10ef0, 0x10ef9),
    (0x11b0a, 0x11b0a),
    (0x11df0, 0x11df1),
    (0x1246f, 0x1246f),
    (0x12475, 0x1247f),
    (0x12550, 0x12686),
    (0x18cd6, 0x18cda),
    (0x18d1f, 0x18d20),
    (0x18e00, 0x19191),
    (0x191a0, 0x191d2),
    (0x1b123, 0x1b128),
    (0x1b168, 0x1b168),
    (0x1d127, 0x1d128),
    (0x1d250, 0x1d252),
    (0x1d25b, 0x1d25c),
    (0x1d25f, 0x1d25f),
    (0x1d280, 0x1d281),
    (0x1d6a6, 0x1d6a6),
    (0x1df1f, 0x1df24),
    (0x1df2b, 0x1df81),
    (0x1df90, 0x1df96),
    (0x1dfcd, 0x1dfff),
    (0x2b81e, 0x2b81e),
    (0x3d000, 0x3fc3f),
];

/// True if `c` is in [`KNOWN_ID_DIFFERENCE`].
fn is_known_id_difference(c: u32) -> bool {
    KNOWN_ID_DIFFERENCE
        .binary_search_by(|&(from, to)| {
            if to < c {
                std::cmp::Ordering::Less
            } else if from > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// True if the front end and RX1 take `c` differently in a group name. Each parser is asked
/// about `c` as the first character of a name (`(?<c>a)`, ID_Start) and as a later one
/// (`(?<ac>a)`, ID_Continue), with `u`. ASCII is not asked (the two tables agree on it, and
/// `>`, `\` or `/` would break the probe), and a surrogate is no character.
fn probe_id_differs(c: u32) -> bool {
    let Some(ch) = char::from_u32(c).filter(|ch| !ch.is_ascii()) else {
        return false;
    };
    [format!("(?<{ch}>a)"), format!("(?<a{ch}>a)")]
        .iter()
        .any(|name| {
            let front = ep9_frontend::check_early_errors(
                &format!("/{name}/u"),
                SourceOpts::script(Strictness::Sloppy),
            )
            .is_ok();
            front != ep9_rx1::parse_str(name, Flags::UNICODE).is_ok()
        })
}

/// The number of `u64` words of one bit per code point U+0000..U+10FFFF.
const SCALAR_WORDS: usize = 0x11_0000 / 64;

/// The kept answers of [`probe_id_differs`]: one `known` and one `differs` bit per code point,
/// a fixed size (2 x 136 KiB) for the whole process (M9-P1 Codex r2 F3).
struct IdAnswers {
    known: Box<[u64; SCALAR_WORDS]>,
    differs: Box<[u64; SCALAR_WORDS]>,
}

impl IdAnswers {
    fn new() -> Self {
        let zero = || -> Box<[u64; SCALAR_WORDS]> {
            vec![0; SCALAR_WORDS]
                .into_boxed_slice()
                .try_into()
                .expect("SCALAR_WORDS words")
        };
        IdAnswers {
            known: zero(),
            differs: zero(),
        }
    }
}

/// True if [`probe_id_differs`] for `c`. ASCII, a surrogate and a value above U+10FFFF are
/// never asked (false); the answer of each other code point is kept for the process in
/// [`IdAnswers`]. Panics if the two parsers take `c` differently and `c` is not in
/// [`KNOWN_ID_DIFFERENCE`]: a new difference (the module doc).
fn id_differs(c: u32) -> bool {
    if c < 0x80 || (0xd800..=0xdfff).contains(&c) || c > 0x10_ffff {
        return false;
    }
    thread_local! {
        static ANSWERS: RefCell<IdAnswers> = RefCell::new(IdAnswers::new());
    }
    let (word, bit) = (c as usize / 64, 1_u64 << (c % 64));
    let differs = ANSWERS.with_borrow_mut(|a| {
        if a.known[word] & bit == 0 {
            a.known[word] |= bit;
            if probe_id_differs(c) {
                a.differs[word] |= bit;
            }
        }
        a.differs[word] & bit != 0
    });
    assert!(
        !differs || is_known_id_difference(c),
        "new front-end/RX1 identifier difference at U+{c:04X}"
    );
    differs
}

/// The start check of [`KNOWN_ID_DIFFERENCE`] (the module doc): the ranges are in order and
/// do not touch; the first, middle and last code point of each range differ; the code point
/// before and after each range does not (if it is not ASCII and not a surrogate).
fn check_known_id_difference() {
    for pair in KNOWN_ID_DIFFERENCE.windows(2) {
        assert!(
            pair[0].0 <= pair[0].1 && pair[0].1 + 1 < pair[1].0,
            "KNOWN_ID_DIFFERENCE is not in order at U+{:04X}",
            pair[1].0
        );
    }
    for &(from, to) in &KNOWN_ID_DIFFERENCE {
        for c in [from, from + (to - from) / 2, to] {
            assert!(
                probe_id_differs(c),
                "KNOWN_ID_DIFFERENCE is stale: no front-end/RX1 identifier difference at U+{c:04X}"
            );
        }
        for c in [from - 1, to + 1] {
            assert!(
                !probe_id_differs(c),
                "new front-end/RX1 identifier difference at U+{c:04X}"
            );
        }
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

/// True if the group name of the text that holds the unit `pos` (the RX1 error position, a
/// UTF-16 unit index; `start <= pos <= end`, so the `>` and the text end count; the names do
/// not overlap) has a code point that [`id_differs`]. The names are those of
/// [`group_names`]: `(?<..>`, and `\k<..>` with `u` or `v` or when the text has a group name.
/// The name is decoded with [`name_char`].
fn group_name_has_id_difference(units: &[u16], flags: Flags, pos: usize) -> bool {
    let with_k = flags.either_unicode() || !group_names(units, false).is_empty();
    let Some((start, end)) = group_names(units, with_k)
        .into_iter()
        .find(|&(start, end)| start <= pos && pos <= end)
    else {
        return false;
    };
    let name = &units[start..end];
    let mut i = 0;
    while i < name.len() {
        let (c, len) = name_char(name, i);
        if c.is_some_and(id_differs) {
            return true;
        }
        i += len;
    }
    false
}

/// The code point at `name[i]` of a group name, and its length in units: raw (a surrogate
/// pair is one code point), `\u{..}` (hex digits only: `u32::from_str_radix` takes a `+`
/// sign, so `\u{+558}` is no code point), a `\uHHHH\uHHHH` surrogate pair or one `\uHHHH`.
/// None for a malformed `\u`, another escape (2 units) or a lone surrogate.
fn name_char(name: &[u16], i: usize) -> (Option<u32>, usize) {
    if !unit_is(name, i, b'\\') {
        return match char::decode_utf16(name[i..].iter().copied()).next() {
            Some(Ok(ch)) => (Some(u32::from(ch)), ch.len_utf16()),
            _ => (None, 1),
        };
    }
    if !unit_is(name, i + 1, b'u') {
        return (None, 2);
    }
    if unit_is(name, i + 2, b'{') {
        let Some(close) = (i + 3..name.len()).find(|&j| unit_is(name, j, b'}')) else {
            return (None, 2);
        };
        let value = String::from_utf16(&name[i + 3..close])
            .ok()
            .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_hexdigit()))
            .and_then(|d| u32::from_str_radix(&d, 16).ok());
        return (value, close + 1 - i);
    }
    match (hex4(name, i + 2), hex4(name, i + 8)) {
        (Some(lead), Some(trail))
            if unit_is(name, i + 6, b'\\')
                && unit_is(name, i + 7, b'u')
                && surrogate_pair(Some(lead), Some(trail)) =>
        {
            let c = 0x1_0000 + ((u32::from(lead) - 0xd800) << 10) + (u32::from(trail) - 0xdc00);
            (Some(c), 12)
        }
        (Some(unit), _) => (Some(u32::from(unit)), 6),
        _ => (None, 2),
    }
}

/// Check 1: the front end and RX1 agree on "valid or not". The one known difference (the
/// module doc): the front end takes the text, RX1 says InvalidCaptureGroupName, and the group
/// name at the RX1 error position has a code point that [`id_differs`] (a difference outside
/// [`KNOWN_ID_DIFFERENCE`] panics there).
fn check_front_end(text: &str, units: &[u16], flags: Flags, rx1_error: Option<&ParseError>) {
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
    let rx1_valid = rx1_error.is_none();
    if rx1_valid == oxc_valid {
        return;
    }
    if rx1_error.is_some_and(|e| {
        e.error == Error::InvalidCaptureGroupName
            && group_name_has_id_difference(units, flags, e.pos)
    }) {
        return;
    }
    panic!("RX1 and the front end disagree: rx1 valid {rx1_valid}, oxc valid {oxc_valid}");
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
/// rows match only their proven form at the RX1 error position, so an RX1 false reject of a
/// valid pattern is still a crash.
fn known_rx0_false_accept(e: &ParseError, units: &[u16], flags: Flags) -> bool {
    match e.error {
        Error::RangeOutOfOrder | Error::NegatedCharacterClassWithStrings => true,
        Error::InvalidCaptureGroupName => has_escape(units, b'u'),
        Error::InvalidCharacterInClass if flags.unicode_sets() => {
            bracket_in_class_string(units, e.pos)
                || bracket_after_set_operator(units, e.pos)
                || amp_after_intersection(units, e.pos)
        }
        Error::OutOfOrderCharacterClass if flags.unicode_sets() => {
            backspace_range_end(units, e.pos, flags)
        }
        _ => false,
    }
}

/// True if `units[..pos]` ends a `v` class range with an escape `\b` (`-\b`, both the `-` and
/// the `\` unescaped) and RX0 refuses the text with that `\b` (and a `\b` range start) written
/// `\x08`: RX0 takes the text only because it reads the `\b` as `b` (the module doc).
fn backspace_range_end(units: &[u16], pos: usize, flags: Flags) -> bool {
    let backspace =
        |i: usize| unit_is(units, i, b'\\') && not_escaped(units, i) && unit_is(units, i + 1, b'b');
    if !(pos >= 3 && unit_is(units, pos - 3, b'-') && not_escaped(units, pos - 3)) {
        return false;
    }
    if !backspace(pos - 2) {
        return false;
    }
    let mut text = Vec::with_capacity(units.len() + 4);
    let mut i = 0;
    while i < units.len() {
        if i == pos - 2 || (pos >= 5 && i == pos - 5 && backspace(i)) {
            text.extend("\\x08".encode_utf16());
            i += 2;
        } else {
            text.push(units[i]);
            i += 1;
        }
    }
    Rx0::compile(&text, ep9_regexp::Flags::from_bits(flags.bits())).is_err()
}

/// True if `units[i]` is the ASCII character `c`.
fn unit_is(units: &[u16], i: usize, c: u8) -> bool {
    units.get(i) == Some(&u16::from(c))
}

/// True if `units[pos]` is an unescaped `[` or `]` inside the body of a `\q{..}` (from `\q{`
/// to the first unescaped `}` that does not end a `\u{..}`).
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
            } else if unit_is(units, i + 1, b'u') && unit_is(units, i + 2, b'{') {
                // `\u{..}`: its `}` does not end the `\q{..}`.
                i += 3;
                while i < pos && !unit_is(units, i, b'}') {
                    i += 1;
                }
                i += 1;
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

/// The group names of the text (the syntax without `u` and `v`), each as `(start, end)`: from
/// the unit after the `<` to the `>` (or the text end). `with_k`: a `\k<` starts one too.
fn group_names(units: &[u16], with_k: bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let (mut j, mut in_class) = (0, false);
    while j < units.len() {
        let group = unit_is(units, j, b'(')
            && unit_is(units, j + 1, b'?')
            && unit_is(units, j + 2, b'<')
            && !unit_is(units, j + 3, b'=')
            && !unit_is(units, j + 3, b'!');
        let k = with_k
            && unit_is(units, j, b'\\')
            && unit_is(units, j + 1, b'k')
            && unit_is(units, j + 2, b'<');
        if !in_class && (group || k) {
            let start = j + 3;
            let end = (start..units.len())
                .find(|&e| unit_is(units, e, b'>'))
                .unwrap_or(units.len());
            out.push((start, end));
            j = end + 1;
        } else if unit_is(units, j, b'\\') {
            j += 2;
        } else {
            if unit_is(units, j, b'[') {
                in_class = true;
            } else if unit_is(units, j, b']') {
                in_class = false;
            }
            j += 1;
        }
    }
    out
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
        || (v && first_operand_before_single_amp(units))
}

/// True if a class of the `v` class text (the outer class or a nested one) has a single `&`
/// (not `&&`) right after its first ClassSetOperand: the operand that regress drops (the module
/// doc). A `&` after a range, after `--`, at the start of a class or inside `\q{..}` does not
/// count. The text is one that RX1 takes.
fn first_operand_before_single_amp(units: &[u16]) -> bool {
    let mut i = 0;
    while i < units.len() {
        if unit_is(units, i, b'\\') {
            // An escaped `[` opens no class; a `\q{..}` is passed whole.
            i = escape_end(units, i);
            continue;
        }
        if unit_is(units, i, b'[') {
            let mut first = i + 1;
            if unit_is(units, first, b'^') {
                first += 1;
            }
            if !unit_is(units, first, b']') {
                let end = operand_end(units, first);
                if unit_is(units, end, b'&') && !unit_is(units, end + 1, b'&') {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// The end (one past) of the ClassSetOperand at `units[i]` of a `v` class: a nested class, an
/// escape, or one character (a surrogate pair is one).
fn operand_end(units: &[u16], i: usize) -> usize {
    if unit_is(units, i, b'\\') {
        return escape_end(units, i);
    }
    if unit_is(units, i, b'[') {
        let (mut j, mut depth) = (i + 1, 1_usize);
        while j < units.len() {
            if unit_is(units, j, b'\\') {
                j = escape_end(units, j);
                continue;
            }
            if unit_is(units, j, b'[') {
                depth += 1;
            } else if unit_is(units, j, b']') {
                depth -= 1;
                if depth == 0 {
                    return j + 1;
                }
            }
            j += 1;
        }
        return units.len();
    }
    if surrogate_pair(units.get(i).copied(), units.get(i + 1).copied()) {
        i + 2
    } else {
        i + 1
    }
}

/// The end (one past) of the escape at `units[i]` (a `\`) of a `v` class: `\q{..}` (its `}`;
/// the escapes inside it are passed whole), `\p{..}`, `\P{..}`, `\u{..}`, `\uHHHH` (with a
/// `\uHHHH` trail surrogate after a lead one: one character), `\xHH`, `\cX`, else 2 units.
fn escape_end(units: &[u16], i: usize) -> usize {
    let n = units.len();
    let braced = |c: u8| unit_is(units, i + 1, c) && unit_is(units, i + 2, b'{');
    if braced(b'q') {
        let mut j = i + 3;
        while j < n && !unit_is(units, j, b'}') {
            j = if unit_is(units, j, b'\\') {
                escape_end(units, j)
            } else {
                j + 1
            };
        }
        return (j + 1).min(n);
    }
    if braced(b'p') || braced(b'P') || braced(b'u') {
        let close = (i + 3..n).find(|&j| unit_is(units, j, b'}'));
        return close.map_or(n, |j| j + 1);
    }
    let end = if unit_is(units, i + 1, b'u') {
        match (hex4(units, i + 2), hex4(units, i + 8)) {
            (Some(lead), Some(trail))
                if unit_is(units, i + 6, b'\\')
                    && unit_is(units, i + 7, b'u')
                    && surrogate_pair(Some(lead), Some(trail)) =>
            {
                i + 12
            }
            (Some(_), _) => i + 6,
            _ => i + 2,
        }
    } else if unit_is(units, i + 1, b'x') {
        i + 4
    } else if unit_is(units, i + 1, b'c') {
        i + 3
    } else {
        i + 2
    };
    end.min(n)
}

/// The 4 hex digits at `units[i..i + 4]` as one code unit.
fn hex4(units: &[u16], i: usize) -> Option<u16> {
    let digits = units.get(i..i + 4)?;
    digits.iter().try_fold(0_u16, |acc, &d| {
        let d = char::from_u32(u32::from(d))?.to_digit(16)?;
        Some(acc * 16 + d as u16)
    })
}

/// True if `lead` and `trail` are a lead and a trail surrogate.
fn surrogate_pair(lead: Option<u16>, trail: Option<u16>) -> bool {
    matches!(lead, Some(0xd800..=0xdbff)) && matches!(trail, Some(0xdc00..=0xdfff))
}

/// Check 3: on a one-class pattern, the RX1 set is the RX0 set. The RX1 set is built for
/// every one-class pattern, also one with a known RX0 set difference: that skip leaves out
/// only the compare, so the RX1 set code still runs (a crash or a hang in it is found; M7-P5
/// Codex R2).
fn check_class_set(units: &[u16], flags: Flags, rx1: &Pattern) {
    let Some(class) = single_class(&rx1.tree) else {
        return;
    };
    // The flags that change what a class matches; `^(?:...)$` must not see `m` or `y`.
    let set_bits = flags.bits()
        & (Flags::IGNORE_CASE
            .union(Flags::DOT_ALL)
            .union(Flags::UNICODE)
            .union(Flags::UNICODE_SETS))
        .bits();
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
    if known_rx0_set_difference(units, flags) {
        return;
    }
    let mut anchored: Vec<u16> = "^(?:".encode_utf16().collect();
    anchored.extend_from_slice(units);
    anchored.extend(")$".encode_utf16());
    let Ok(program) = Rx0::compile(&anchored, ep9_regexp::Flags::from_bits(set_bits)) else {
        panic!("RX0 compiles the class but not ^(?:class)$");
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

libfuzzer_sys::fuzz_target!(init: check_known_id_difference(), |data: &[u8]| {
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
    check_front_end(text, &units, flags, rx1.as_ref().err());
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
