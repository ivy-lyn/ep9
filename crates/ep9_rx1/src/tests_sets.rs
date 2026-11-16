//! Unit tests of port step 2 (the class sets): membership against node (V8), the V8
//! algorithms on small lists, the case data, and the tables.

use super::*;
use crate::special_case::Mode;

fn flags(s: &str) -> Flags {
    let units: Vec<u16> = s.encode_utf16().collect();
    Flags::parse(&units).expect("valid flags")
}

fn r(from: u32, to: u32) -> CharacterRange {
    CharacterRange::range(from, to)
}

/// The probe code points of [`NODE_SETS`].
static PROBES: &[u32] = &[
    0x9, 0x20, 0x30, 0x39, 0x41, 0x4B, 0x53, 0x5F, 0x61, 0x6B, 0x73, 0x7F, 0x80, 0xA0, 0xB5, 0xC0,
    0xDF, 0xE0, 0xFF, 0x130, 0x131, 0x178, 0x17F, 0x1C4, 0x1C5, 0x1C6, 0x345, 0x390, 0x399, 0x39C,
    0x3A3, 0x3A9, 0x3B9, 0x3BC, 0x3C2, 0x3C3, 0x3C9, 0x3F4, 0x1E9E, 0x1F80, 0x1F88, 0x1FBE, 0x2028,
    0x2126, 0x212A, 0x212B, 0xD800, 0xDBFF, 0xDC00, 0xDFFF, 0xFEFF, 0xFFFF, 0x10000, 0x10400,
    0x10428, 0x1D400, 0x1F600, 0xE0001, 0x10FFFE,
];

/// (pattern, flags, one `1` for each probe of [`PROBES`] that `/^(?:pattern)$/flags` matches)
/// from node v26.13.0 (`work/node/sets.mjs` in the RX1-1 evidence).
static NODE_SETS: &[(&str, &str, &str)] = &[
    (
        "\\w",
        "ui",
        "00111111111000000000001000000000000000000000100000000000000",
    ),
    (
        "\\W",
        "ui",
        "11000000000111111111110111111111111111111111011111111111111",
    ),
    (
        "[\\w]",
        "ui",
        "00111111111000000000001000000000000000000000100000000000000",
    ),
    (
        "[\\W]",
        "ui",
        "11000000000111111111110111111111111111111111011111111111111",
    ),
    (
        "\\W",
        "i",
        "11000000000111111111111111111111111111111111111111110000000",
    ),
    (
        "\\w",
        "vi",
        "00111111111000000000001000000000000000000000100000000000000",
    ),
    (
        "\\W",
        "vi",
        "11000000000111111111110111111111111111111111011111111111111",
    ),
    (
        "[^\\W]",
        "ui",
        "00111111111000000000001000000000000000000000100000000000000",
    ),
    (
        "[a-z]",
        "i",
        "00001110111000000000000000000000000000000000000000000000000",
    ),
    (
        "[a-z]",
        "ui",
        "00001110111000000000001000000000000000000000100000000000000",
    ),
    (
        "[a-z]",
        "vi",
        "00001110111000000000001000000000000000000000100000000000000",
    ),
    (
        "[\\u00e0-\\u00ff]",
        "i",
        "00000000000000010110010000000000000000000000000000000000000",
    ),
    (
        "\\u00df",
        "i",
        "00000000000000001000000000000000000000000000000000000000000",
    ),
    (
        "\\u1e9e",
        "i",
        "00000000000000000000000000000000000000100000000000000000000",
    ),
    (
        "\\u1e9e",
        "ui",
        "00000000000000001000000000000000000000100000000000000000000",
    ),
    (
        "[\\u1e9e]",
        "i",
        "00000000000000000000000000000000000000100000000000000000000",
    ),
    (
        "[\\u1e9e]",
        "ui",
        "00000000000000001000000000000000000000100000000000000000000",
    ),
    (
        "k",
        "i",
        "00000100010000000000000000000000000000000000000000000000000",
    ),
    (
        "k",
        "ui",
        "00000100010000000000000000000000000000000000100000000000000",
    ),
    (
        "s",
        "ui",
        "00000010001000000000001000000000000000000000000000000000000",
    ),
    (
        "\\u03c3",
        "i",
        "00000000000000000000000000000010001100000000000000000000000",
    ),
    (
        "\\u03c3",
        "ui",
        "00000000000000000000000000000010001100000000000000000000000",
    ),
    (
        "\\u0399",
        "i",
        "00000000000000000000000000101000100000000100000000000000000",
    ),
    (
        "\\u0399",
        "ui",
        "00000000000000000000000000101000100000000100000000000000000",
    ),
    (
        "\\u1f80",
        "i",
        "00000000000000000000000000000000000000010000000000000000000",
    ),
    (
        "\\u1f80",
        "ui",
        "00000000000000000000000000000000000000011000000000000000000",
    ),
    (
        "\\u00b5",
        "i",
        "00000000000000100000000000000100010000000000000000000000000",
    ),
    (
        "\\u00b5",
        "ui",
        "00000000000000100000000000000100010000000000000000000000000",
    ),
    (
        "[^a]",
        "i",
        "11110111011111111111111111111111111111111111111111110000000",
    ),
    (
        "[^a]",
        "ui",
        "11110111011111111111111111111111111111111111111111111111111",
    ),
    (
        "[^a]",
        "vi",
        "11110111011111111111111111111111111111111111111111111111111",
    ),
    (
        "\\p{Lu}",
        "u",
        "00001110000000010001010100001111000001100001110000000101000",
    ),
    (
        "\\p{Lu}",
        "ui",
        "00001110111000111111011111101111111111100101110000000111000",
    ),
    (
        "\\p{Lu}",
        "vi",
        "00001110111000111111011111101111111111100101110000000111000",
    ),
    (
        "\\P{Lu}",
        "ui",
        "11111111111111111110111111111111111111111111111111111110111",
    ),
    (
        "\\P{Ll}",
        "ui",
        "11111111111111111111011111101111111111111111111111111111111",
    ),
    (
        "\\P{Ll}",
        "vi",
        "11110001000111000001000000000000000000000010001111111001111",
    ),
    (
        "[^\\P{Ll}]",
        "ui",
        "00000000000000000000100000010000000000000000000000000000000",
    ),
    (
        "[\\p{Lu}--[A-Z]]",
        "vi",
        "00000000000000111111010111101111111111100101010000000111000",
    ),
    (
        "[\\p{Lu}--[A-Z]]",
        "v",
        "00000000000000010001010100001111000001100001110000000101000",
    ),
    (
        "[\\p{L}&&\\p{sc=Greek}]",
        "v",
        "00000000000000000000000000011111111111011101000000000000000",
    ),
    (
        "[\\p{L}&&\\p{sc=Greek}]",
        "vi",
        "00000000000000100000000000111111111111011101000000000000000",
    ),
    (
        "\\p{ASCII}",
        "ui",
        "11111111111100000000001000000000000000000000100000000000000",
    ),
    (
        "\\p{Any}",
        "u",
        "11111111111111111111111111111111111111111111111111111111111",
    ),
    (
        "\\P{Any}",
        "u",
        "00000000000000000000000000000000000000000000000000000000000",
    ),
    (
        "\\p{Assigned}",
        "u",
        "11111111111111111111111111111111111111111111111111101111110",
    ),
    (
        "\\P{Assigned}",
        "vi",
        "00000000000000000000000000000000000000000000000000010000001",
    ),
    (
        "\\p{scx=Grek}",
        "u",
        "00000000000000000000000000111111111111011101000000000000000",
    ),
    (
        "\\p{sc=Grek}",
        "u",
        "00000000000000000000000000011111111111011101000000000000000",
    ),
    (
        "\\p{Lowercase}",
        "u",
        "00000000111000101110101001110000111110010100000000000010000",
    ),
    (
        "\\p{Lowercase}",
        "vi",
        "00001110111000111110111111111111111111111101110000000110000",
    ),
    (
        "\\p{White_Space}",
        "u",
        "11000000000001000000000000000000000000000010000000000000000",
    ),
    (
        "\\s",
        "u",
        "11000000000001000000000000000000000000000010000000100000000",
    ),
    (
        "\\S",
        "u",
        "00111111111110111111111111111111111111111101111111011111111",
    ),
    (
        "\\d",
        "ui",
        "00110000000000000000000000000000000000000000000000000000000",
    ),
    (
        "\\D",
        "vi",
        "11001111111111111111111111111111111111111111111111111111111",
    ),
    (
        ".",
        "u",
        "11111111111111111111111111111111111111111101111111111111111",
    ),
    (
        ".",
        "su",
        "11111111111111111111111111111111111111111111111111111111111",
    ),
    (
        ".",
        "",
        "11111111111111111111111111111111111111111101111111110000000",
    ),
    (
        ".",
        "s",
        "11111111111111111111111111111111111111111111111111110000000",
    ),
    (
        "[^]",
        "v",
        "11111111111111111111111111111111111111111111111111111111111",
    ),
    (
        "[]",
        "v",
        "00000000000000000000000000000000000000000000000000000000000",
    ),
    (
        "[^]",
        "",
        "11111111111111111111111111111111111111111111111111110000000",
    ),
    (
        "[]",
        "",
        "00000000000000000000000000000000000000000000000000000000000",
    ),
    (
        "[\\0-\\uFFFE]",
        "",
        "11111111111111111111111111111111111111111111111111100000000",
    ),
    (
        "[^\\0-\\uFFFE]",
        "",
        "00000000000000000000000000000000000000000000000000010000000",
    ),
    (
        "[\\ud800-\\udfff]",
        "u",
        "00000000000000000000000000000000000000000000001111000000000",
    ),
    (
        "[^\\ud800]",
        "u",
        "11111111111111111111111111111111111111111111110111111111111",
    ),
    (
        "\\ud800",
        "u",
        "00000000000000000000000000000000000000000000001000000000000",
    ),
    (
        "[\\q{a|K|bc}]",
        "v",
        "00000100100000000000000000000000000000000000000000000000000",
    ),
    (
        "\\u{10400}",
        "ui",
        "00000000000000000000000000000000000000000000000000000110000",
    ),
    (
        "[\\u{10400}]",
        "u",
        "00000000000000000000000000000000000000000000000000000100000",
    ),
    (
        "[\\u{10400}]",
        "ui",
        "00000000000000000000000000000000000000000000000000000110000",
    ),
    (
        "\\p{Emoji_Presentation}",
        "v",
        "00000000000000000000000000000000000000000000000000000000100",
    ),
    (
        "\\p{Basic_Emoji}",
        "v",
        "00000000000000000000000000000000000000000000000000000000100",
    ),
    (
        "[\\p{Basic_Emoji}--\\q{\\u{1F600}}]",
        "v",
        "00000000000000000000000000000000000000000000000000000000000",
    ),
    (
        "\\p{Changes_When_NFKC_Casefolded}",
        "u",
        "00001110000001111001011111101111001001111101110000100101010",
    ),
    (
        "\\p{Bidi_M}",
        "u",
        "00000000000000000000000000000000000000000000000000000000000",
    ),
    (
        "\\p{gc=Cn}",
        "u",
        "00000000000000000000000000000000000000000000000000010000001",
    ),
    (
        "\\p{gc=LC}",
        "u",
        "00001110111000111111111111011111111111111101110000000111000",
    ),
    (
        "\\p{C}",
        "u",
        "10000000000110000000000000000000000000000000001111110000011",
    ),
    (
        "[\\p{Ll}\\p{Lt}]",
        "ui",
        "00001110111000111110111111111111111111111101110000000110000",
    ),
    (
        "[^\\p{Ll}\\p{Lt}]",
        "ui",
        "11110001000111000001000000000000000000000010001111111001111",
    ),
    (
        "\\p{Lt}",
        "vi",
        "00000000000000000000000111000000000000011000000000000000000",
    ),
    (
        "\\p{Lt}",
        "ui",
        "00000000000000000000000111000000000000011000000000000000000",
    ),
    (
        "\\u01c5",
        "i",
        "00000000000000000000000111000000000000000000000000000000000",
    ),
    (
        "\\u01c5",
        "ui",
        "00000000000000000000000111000000000000000000000000000000000",
    ),
];

/// Spec rows where node v26.13.0 differs from ECMA-262 and from the V8 code at the port
/// commit: (pattern, flags, code point, the spec result). Each is a `v`+`i` fold that node's
/// V8 misses. `\q{K}`: MaybeSimpleCaseFolding gives `k`, and CharacterSetMatcher compares
/// Canonicalize values, so `K` and U+212A match (node: no; V8 at the port commit adds the
/// closure in AddClassString). `\p{ASCII}`: scf(U+017F) = `s` and scf(U+212A) = `k` are in
/// MaybeSimpleCaseFolding(ASCII), so both match (node: no; V8 at the port commit closes ASCII
/// in AddUnicodeSetRanges). The other probes of these patterns are in [`NODE_SETS`] form
/// below.
static SPEC_NOT_NODE: &[(&str, &str, u32, bool)] = &[
    ("[\\q{a|K|bc}]", "vi", 0x41, true),
    ("[\\q{a|K|bc}]", "vi", 0x4b, true),
    ("[\\q{a|K|bc}]", "vi", 0x212a, true),
    ("\\p{ASCII}", "vi", 0x17f, true),
    ("\\p{ASCII}", "vi", 0x212a, true),
    ("\\P{ASCII}", "vi", 0x17f, false),
    ("\\P{ASCII}", "vi", 0x212a, false),
];

/// The node rows of the [`SPEC_NOT_NODE`] patterns (node's bits; the spec rows override).
static NODE_SETS_SPEC: &[(&str, &str, &str)] = &[
    (
        "\\p{ASCII}",
        "vi",
        "11111111111100000000000000000000000000000000000000000000000",
    ),
    (
        "\\P{ASCII}",
        "vi",
        "00000000000011111111111111111111111111111111111111111111111",
    ),
    (
        "[\\q{a|K|bc}]",
        "vi",
        "00000000110000000000000000000000000000000000000000000000000",
    ),
];

/// The top class of a one-class pattern, as a membership function of one code point (a code
/// unit without `u`/`v`; a larger value never matches then).
fn rx1_member(pattern: &str, f: Flags) -> impl Fn(u32) -> bool {
    let p = parse_str(pattern, f).unwrap_or_else(|e| panic!("/{pattern}/: {e:?}"));
    let unicode = f.either_unicode();
    let tree = p.tree;
    move |c: u32| {
        if !unicode && c > 0xffff {
            return false;
        }
        match &tree {
            Tree::ClassRanges(cr) => class_ranges(cr, f, false).matches(c),
            t @ (Tree::ClassSetOperand(_) | Tree::ClassSetExpression(_)) => {
                contains(&class_set(t, f).unwrap().ranges, c)
            }
            Tree::Atom(units) if units.len() == 1 => {
                if f.ignore_case() {
                    case_independent_letters(units[0], unicode, false)
                        .contains(&u16::try_from(c).unwrap_or(0xffff))
                        && c <= 0xffff
                } else {
                    c == u32::from(units[0])
                }
            }
            Tree::Atom(units) if units.len() == 2 => {
                let cp = 0x10000
                    + ((u32::from(units[0]) - 0xd800) << 10)
                    + (u32::from(units[1]) - 0xdc00);
                c == cp
            }
            other => panic!("/{pattern}/: not one class: {other:?}"),
        }
    }
}

#[test]
fn class_sets_match_node() {
    let mut bad = Vec::new();
    for (table, spec) in [(NODE_SETS, false), (NODE_SETS_SPEC, true)] {
        for &(pattern, f, bits) in table {
            assert_eq!(bits.len(), PROBES.len(), "{pattern}");
            let member = rx1_member(pattern, flags(f));
            for (&c, want) in PROBES.iter().zip(bits.bytes()) {
                let mut want = want == b'1';
                if spec {
                    let row = SPEC_NOT_NODE
                        .iter()
                        .find(|&&(p, fl, cp, _)| p == pattern && fl == f && cp == c);
                    if let Some(&(_, _, _, v)) = row {
                        assert_ne!(v, want, "/{pattern}/{f} U+{c:04X}: not a node difference");
                        want = v;
                    }
                }
                if member(c) != want {
                    bad.push(format!("/{pattern}/{f} U+{c:04X}: want {want}"));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} mismatches:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// The strings of `v`-mode classes (node v26.13.0: `/^(?:pattern)$/v.test(s)`).
#[test]
fn class_strings_match_node() {
    type Case = (&'static str, &'static str, &'static [(&'static str, bool)]);
    let cases: &[Case] = &[
        (
            "[\\q{a|K|bc}]",
            "vi",
            &[
                ("bc", true),
                ("BC", true),
                ("Bc", true),
                ("b", false),
                ("ab", false),
            ],
        ),
        (
            "\\p{RGI_Emoji}",
            "v",
            &[
                ("\u{1F1FA}\u{1F1F8}", true),
                ("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", true),
                ("#\u{FE0F}\u{20E3}", true),
                ("\u{1F600}", true),
                ("a", false),
                ("\u{1F44D}\u{1F3FB}", true),
            ],
        ),
        (
            "\\p{RGI_Emoji_Flag_Sequence}",
            "v",
            &[
                ("\u{1F1FA}\u{1F1F8}", true),
                ("\u{1F1FA}\u{1F1FA}", false),
                ("\u{1F1E6}", false),
            ],
        ),
        (
            "\\p{Emoji_Keycap_Sequence}",
            "v",
            &[
                ("#\u{FE0F}\u{20E3}", true),
                ("1\u{FE0F}\u{20E3}", true),
                ("#\u{20E3}", false),
            ],
        ),
    ];
    for &(pattern, f, strings) in cases {
        let f = flags(f);
        let p = parse_str(pattern, f).unwrap();
        let set = class_set(&p.tree, f).expect("a v-mode class");
        for &(s, want) in strings {
            let cps: Vec<u32> = s.chars().map(u32::from).collect();
            let got = if cps.len() == 1 {
                contains(&set.ranges, cps[0])
            } else {
                let key: Vec<u32> = if f.ignore_case() {
                    cps.iter().map(|&c| simple_fold(c)).collect()
                } else {
                    cps
                };
                set.strings.contains_key(&ClassString(key))
            };
            assert_eq!(got, want, "/{pattern}/ {s:?}");
        }
    }
}

/// ECMA-262 CharacterComplement keeps U+10FFFF; V8's Negate drops it (node v26.13.0:
/// `/[^\0-\u{10FFFE}]/u.test("\u{10FFFF}")` is false). RX1 follows the spec (charset.rs).
#[test]
fn the_complement_keeps_the_last_code_point() {
    let member = rx1_member("[^\\0-\\u{10FFFE}]", flags("u"));
    assert!(member(0x10_ffff));
    assert!(!member(0x10_fffe));
    assert_eq!(negate(&[r(0, 0x10_fffe)]), vec![r(0x10_ffff, 0x10_ffff)]);
    assert_eq!(negate(&[r(0, 0x10_ffff)]), vec![]);
    assert_eq!(negate(&[]), vec![CharacterRange::everything()]);
    let set = class_set(
        &parse_str("[^\\0-\\u{10FFFE}]", flags("v")).unwrap().tree,
        flags("v"),
    )
    .unwrap();
    assert_eq!(set.ranges, vec![r(0x10_ffff, 0x10_ffff)]);
}

#[test]
fn canonicalize_negate_intersect_subtract() {
    let mut v = vec![r(10, 20), r(0, 5), r(6, 8), r(19, 30), r(40, 40)];
    canonicalize(&mut v);
    assert_eq!(v, vec![r(0, 8), r(10, 30), r(40, 40)]);
    assert!(is_canonical(&v));
    assert!(!is_canonical(&[r(0, 5), r(6, 7)]));
    assert_eq!(negate(&v), vec![r(9, 9), r(31, 39), r(41, MAX_CODE_POINT)]);
    let b = vec![r(5, 12), r(25, 45)];
    assert_eq!(
        intersect(&v, &b),
        vec![r(5, 8), r(10, 12), r(25, 30), r(40, 40)]
    );
    assert!(intersects(&v, &b));
    assert!(!intersects(&v, &[r(9, 9), r(31, 39)]));
    assert_eq!(subtract(&v, &b), vec![r(0, 4), r(13, 24)]);
    assert_eq!(subtract(&v, &[]), v);
    assert_eq!(subtract(&[], &b), vec![]);
    // Each of V8's six Subtract cases, (a) to (f).
    assert_eq!(subtract(&[r(10, 20)], &[r(0, 5)]), vec![r(10, 20)]);
    assert_eq!(
        subtract(&[r(10, 20), r(30, 31)], &[r(25, 26)]),
        vec![r(10, 20), r(30, 31)]
    );
    assert_eq!(subtract(&[r(10, 20)], &[r(5, 25)]), vec![]);
    assert_eq!(
        subtract(&[r(10, 20)], &[r(12, 14)]),
        vec![r(10, 11), r(15, 20)]
    );
    assert_eq!(subtract(&[r(10, 20)], &[r(15, 25)]), vec![r(10, 14)]);
    assert_eq!(subtract(&[r(10, 20)], &[r(5, 15)]), vec![r(16, 20)]);
    let mut one = vec![r(0x41, 0x5a), r(0x100, 0x200)];
    clamp_to_one_byte(&mut one);
    assert_eq!(one, vec![r(0x41, 0x5a)]);
    let mut cross = vec![r(0x41, 0x200)];
    clamp_to_one_byte(&mut cross);
    assert_eq!(cross, vec![r(0x41, 0xff)]);
}

/// A brute-force model of the four list operations on random small lists (a fixed seed).
#[test]
fn list_operations_equal_a_bit_model() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    const N: u32 = 64;
    let to_bits = |v: &[CharacterRange]| -> u64 {
        v.iter()
            .flat_map(|x| x.from()..=x.to().min(N - 1))
            .fold(0u64, |b, c| b | (1 << c))
    };
    for _ in 0..2000 {
        let mut lists = [Vec::new(), Vec::new()];
        for l in &mut lists {
            for _ in 0..(next() % 5) {
                let a = (next() % u64::from(N)) as u32;
                let b = (next() % u64::from(N)) as u32;
                l.push(r(a.min(b), a.max(b)));
            }
            canonicalize(l);
            assert!(is_canonical(l));
        }
        let (a, b) = (to_bits(&lists[0]), to_bits(&lists[1]));
        assert_eq!(to_bits(&intersect(&lists[0], &lists[1])), a & b);
        assert_eq!(to_bits(&subtract(&lists[0], &lists[1])), a & !b);
        assert_eq!(to_bits(&negate(&lists[0])), !a);
        assert_eq!(intersects(&lists[0], &lists[1]), a & b != 0);
    }
}

#[test]
fn case_data_follows_v8_and_the_spec() {
    // Without u: Canonicalize. U+017F and U+212A match only themselves (their uppercase is
    // ASCII or themselves); U+00DF uppercases to "SS".
    assert_eq!(
        case_independent_letters(0x6b, false, false),
        vec![0x4b, 0x6b]
    );
    assert_eq!(case_independent_letters(0x212a, false, false), vec![0x212a]);
    assert_eq!(case_independent_letters(0x17f, false, false), vec![0x17f]);
    assert_eq!(case_independent_letters(0xdf, false, false), vec![0xdf]);
    assert_eq!(
        case_independent_letters(0xff, false, false),
        vec![0xff, 0x178]
    );
    assert_eq!(case_independent_letters(0x178, false, true), vec![0xff]);
    // With u: Simple_Case_Folding.
    assert_eq!(
        case_independent_letters(0x6b, true, false),
        vec![0x4b, 0x6b, 0x212a]
    );
    assert_eq!(
        case_independent_letters(0xdf, true, false),
        vec![0xdf, 0x1e9e]
    );
    assert_eq!(
        case_independent_letters(0x3b8, true, false),
        vec![0x398, 0x3b8, 0x3d1, 0x3f4]
    );
    assert_eq!(simple_fold(0x41), 0x61);
    assert_eq!(simple_fold(0x1e9e), 0xdf);
    assert_eq!(simple_fold(0x10400), 0x10428);
    // U+212A is in IgnoreSet: without u it matches only itself, so it is its own key.
    assert_eq!(equivalence_key(0x212a, Mode::NonUnicode), 0x212a);
    assert_eq!(equivalence_key(0x4b, Mode::NonUnicode), 0x6b);
    assert_eq!(equivalence_key(0xdf, Mode::NonUnicode), 0xdf);
    assert_eq!(equivalence_key(0x1e9e, Mode::NonUnicode), 0x1e9e);
    assert_eq!(equivalence_key(0x1e9e, Mode::Unicode), 0xdf);
    // Unicode 17.0.0 has no BMP character with a supplementary equivalent (V8: from 18.0).
    assert!((0..=0xffffu16).all(|c| !has_supplementary_equivalents(c)));
    // The non-Unicode closure of a range: [a-z] gets A-Z, nothing else.
    let mut v = vec![r(0x61, 0x7a)];
    add_case_equivalents(&mut v, false);
    canonicalize(&mut v);
    assert_eq!(v, vec![r(0x41, 0x5a), r(0x61, 0x7a)]);
    // One byte: U+0178 (outside Latin-1) still gives U+00FF.
    let mut v = vec![r(0x178, 0x178)];
    add_case_equivalents(&mut v, true);
    canonicalize(&mut v);
    assert_eq!(v, vec![r(0xff, 0xff), r(0x178, 0x178)]);
    let mut v = vec![r(0x61, 0x7a)];
    add_unicode_case_equivalents(&mut v);
    assert_eq!(
        v,
        vec![
            r(0x41, 0x5a),
            r(0x61, 0x7a),
            r(0x17f, 0x17f),
            r(0x212a, 0x212a)
        ]
    );
}

/// Every Simple_Case_Folding class: each member folds to one value, and the closure of any
/// member is the class (the CASE_ORBIT cycles are closed and agree with SIMPLE_FOLDING).
#[test]
fn the_case_orbits_are_the_folding_classes() {
    use crate::unicode_sets::{CASE_ORBIT, SIMPLE_FOLDING};
    let mut seen = 0;
    for &(c, _) in CASE_ORBIT {
        let mut set = vec![CharacterRange::singleton(c)];
        special_case::close_over(&mut set, Mode::Unicode);
        let members: Vec<u32> = set.iter().flat_map(|x| x.from()..=x.to()).collect();
        assert!(members.len() >= 2, "U+{c:04X}");
        let f = simple_fold(c);
        assert!(members.iter().all(|&m| simple_fold(m) == f), "U+{c:04X}");
        assert!(members.contains(&f), "U+{c:04X}");
        seen += 1;
    }
    assert_eq!(seen, CASE_ORBIT.len());
    for &(c, f) in SIMPLE_FOLDING {
        assert!(
            CASE_ORBIT.binary_search_by_key(&c, |x| x.0).is_ok(),
            "U+{c:04X}"
        );
        assert!(
            CASE_ORBIT.binary_search_by_key(&f, |x| x.0).is_ok(),
            "U+{f:04X}"
        );
    }
}

/// Every property value that the parser takes has a set, and the two generated files come
/// from one UCD version.
#[test]
fn every_parsed_property_name_has_a_set() {
    use crate::unicode_tables::{GENERAL_CATEGORY, SCRIPT};
    assert_eq!(crate::unicode_sets::VERSION, UCD_VERSION);
    for &(alias, _) in GENERAL_CATEGORY {
        let p = parse_str(&format!("\\p{{gc={alias}}}"), flags("u")).unwrap();
        let Tree::ClassRanges(cr) = &p.tree else {
            panic!("{alias}")
        };
        let _ = class_ranges(cr, flags("u"), false);
    }
    for &(alias, short) in SCRIPT {
        for name in ["sc", "scx"] {
            let p = parse_str(&format!("\\p{{{name}={alias}}}"), flags("u")).unwrap();
            let Tree::ClassRanges(cr) = &p.tree else {
                panic!("{alias}")
            };
            let set = class_ranges(cr, flags("u"), false);
            // Only Katakana_Or_Hiragana is empty (ECMA-262 takes it; V8 refuses it).
            assert_eq!(
                set.ranges.is_empty() || set.negated,
                short == "Hrkt",
                "{name}={alias}"
            );
        }
    }
    for name in crate::unicode::binary_names() {
        let p = parse_str(&format!("\\p{{{name}}}"), flags("u")).unwrap();
        let Tree::ClassRanges(cr) = &p.tree else {
            panic!("{name}")
        };
        assert!(
            !class_ranges(cr, flags("u"), false).ranges.is_empty(),
            "{name}"
        );
    }
    for name in [
        "Basic_Emoji",
        "Emoji_Keycap_Sequence",
        "RGI_Emoji",
        "RGI_Emoji_Flag_Sequence",
        "RGI_Emoji_Modifier_Sequence",
        "RGI_Emoji_Tag_Sequence",
        "RGI_Emoji_ZWJ_Sequence",
    ] {
        let p = parse_str(&format!("\\p{{{name}}}"), flags("v")).unwrap();
        let set = class_set(&p.tree, flags("v")).unwrap();
        assert!(!set.strings.is_empty(), "{name}");
    }
}

/// With `u`+`i`, a supplementary character with case equivalents is a one-character class in
/// the tree (V8 NeedsDesugaringForIgnoreCase); without `i`, or without equivalents, it is an
/// atom.
#[test]
fn supplementary_characters_with_case_equivalents_are_classes() {
    let one = |s: &str, f: &str| parse_str(s, flags(f)).unwrap().tree;
    assert!(matches!(one("\\u{10400}", "ui"), Tree::ClassRanges(_)));
    assert!(matches!(one("\\u{10400}", "vi"), Tree::ClassRanges(_)));
    assert_eq!(one("\\u{10400}", "u"), Tree::Atom(vec![0xd801, 0xdc00]));
    assert_eq!(one("\\u{1F600}", "ui"), Tree::Atom(vec![0xd83d, 0xde00]));
    assert_eq!(one("a", "ui"), Tree::Atom(vec![0x61]));
    // Its set is the pair (U+10400, U+10428), as node matches (NODE_SETS).
    let Tree::ClassRanges(cr) = one("\\u{10400}", "ui") else {
        unreachable!()
    };
    let set = class_ranges(&cr, flags("ui"), false);
    assert_eq!(set.ranges, vec![r(0x10400, 0x10400), r(0x10428, 0x10428)]);
}

/// The class-set walk recurses once per nested `v`-mode class: at the nesting cap it fits a
/// 1 MiB thread stack.
#[test]
fn the_class_set_walk_fits_the_nesting_cap() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let depth = MAX_NESTING - 1;
            let classes = format!("{}a{}", "[".repeat(depth), "]".repeat(depth));
            let p = parse_str(&classes, flags("vi")).unwrap();
            let set = class_set(&p.tree, flags("vi")).unwrap();
            assert_eq!(set.ranges, vec![r(0x41, 0x41), r(0x61, 0x61)]);
        })
        .unwrap()
        .join()
        .unwrap();
}

/// RX1-SUB-QUAD: `[A--B1--B2--...]` with n operands, A of n strings and a range set. The
/// subtraction visits each element a fixed number of times (the old loop rescanned A per
/// operand: n * n). A count of the visits, not a time bound.
#[test]
fn class_subtraction_work_grows_linearly() {
    let n = 16 * 1024;
    let mut p = String::from("[[\\p{L}\\q{keep");
    for k in 0..n {
        p.push_str(&format!("|x{k}"));
    }
    p.push_str("}]");
    for k in 0..n {
        p.push_str(&format!("--[\\q{{x{k}}}\\u{{{:X}}}]", 0x4e00 + k));
    }
    p.push(']');
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let parsed = parse_str(&p, flags("v")).unwrap();
            crate::sets::SUBTRACTION_WORK.with(|w| w.set(0));
            let set = class_set(&parsed.tree, flags("v")).unwrap();
            let work = crate::sets::SUBTRACTION_WORK.with(std::cell::Cell::get);
            let letters = class_set(&parse_str("[\\p{L}]", flags("v")).unwrap().tree, flags("v"))
                .unwrap()
                .ranges;
            let removed = [r(0x4e00, 0x4e00 + n - 1)];
            assert_eq!(set.ranges, crate::charset::subtract(&letters, &removed));
            let keys: Vec<Vec<u32>> = set.strings.keys().map(|k| k.0.clone()).collect();
            assert_eq!(
                keys,
                vec!["keep".chars().map(u32::from).collect::<Vec<_>>()]
            );
            // About 2n operand elements, plus A's ranges and the union once.
            let bound = 4 * n as usize + 4 * letters.len();
            assert!(work <= bound, "{work} visits for n = {n} (bound {bound})");
        })
        .unwrap()
        .join()
        .unwrap();
}
