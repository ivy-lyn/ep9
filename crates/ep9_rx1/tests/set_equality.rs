//! The set-equality test: the code point sets of RX1 port step 2 against RX0 on the RegExp
//! sources of test262 (plan v12 section 8.3 RX1-1, "the RX0 diff test").
//!
//! ```text
//! test262 pairs (tests/corpus: built-ins/RegExp, language/literals/regexp)
//!   -> RX1 parses it and regress parses the whole pattern (else not compared; parse_equality
//!      covers the verdicts)
//!   -> each class of the tree, with the flags in force (modifier groups too):
//!        ClassRanges           -> ep9_rx1::class_ranges   (code points, negated)
//!        ClassSetOperand/Expr  -> ep9_rx1::class_set      (code points, strings)
//!        Atom units under i    -> ep9_rx1::case_independent_letters
//!   -> one key per (class text, i u v s): the class printed back from the tree
//!   -> regress compiles the class text alone and runs over the probe subject;
//!      each probe is a member when regress matches it
//!   -> every probe where RX1 and regress differ, grouped by key, == tests/set_equality_known.txt
//! ```
//!
//! RX0 is regress (the fork of the root `[patch.crates-io]`) without `ep9_regexp`'s pre-pass,
//! as in the parse-equality test. A class without strings is compared on every code point
//! (every code unit without `u`/`v`): regress searches one subject that holds each of them
//! once, and each match must be one character long. A class with strings (`v` mode) is
//! compared one probe at a time (`^(?:class)$`): all of U+0000-U+02FF, every 61st code point,
//! each code point that Simple_Case_Folding changes and its fold, and the edges of each RX1
//! range (from - 1, from, to, to + 1); and each of its strings, the string without its last
//! code point, and the string with `x` after it. An atom character under `i` is compared on
//! every code unit, as a class.
//!
//! What it trusts: the class text comes from RX1's own tree (escapes for every code point; a
//! lone escape such as `\W` outside a class stays bare), so the test checks the compiler's
//! sets, not the parser's tree (the parse tests check that). For a class with strings, a
//! code point that is no probe is not compared. The test fails if the counts fall under the
//! floors, so a broken walk cannot pass with few classes.
//!
//! Run: `bash scripts/sync-test262.sh`, then
//! `cargo test --locked --release -p ep9_rx1 --test set_equality -- --ignored --nocapture`.
//! Set `EP9_RX1_SETS=<file>` to write each class without strings and RX1's members (JSON
//! lines) for a check against node.

mod corpus;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use corpus::{escape, test262_pairs};
use ep9_rx1::{
    ClassItem, ClassSetExpression, ClassSetOperand, ClassSetOperation, ClassString, Flags,
    Property, StandardSet, Tree, case_independent_letters, class_ranges, class_set, simple_fold,
};

/// One class to compare: its text for regress, the flags in force, and RX1's node.
struct Class {
    text: Vec<u32>,
    flags: Flags,
    node: Tree,
}

/// The flags that change what a class matches.
fn set_flags(f: Flags) -> Flags {
    Flags::empty()
        .with(Flags::IGNORE_CASE, f.ignore_case())
        .with(Flags::UNICODE, f.contains(Flags::UNICODE))
        .with(Flags::UNICODE_SETS, f.unicode_sets())
        .with(Flags::DOT_ALL, f.dot_all())
}

// ---- printing a class back from the tree ----------------------------------------------------

fn push_str(out: &mut Vec<u32>, s: &str) {
    out.extend(s.chars().map(u32::from));
}

/// One code point: a lone surrogate as itself (an escape could join two of them into a pair
/// in regress), any other as an escape.
fn push_char(out: &mut Vec<u32>, c: u32, unicode: bool) {
    if (0xd800..=0xdfff).contains(&c) {
        out.push(c);
    } else if unicode {
        push_str(out, &format!("\\u{{{c:X}}}"));
    } else {
        push_str(out, &format!("\\u{c:04X}"));
    }
}

fn standard(s: StandardSet) -> &'static str {
    match s {
        StandardSet::Whitespace => "\\s",
        StandardSet::NotWhitespace => "\\S",
        StandardSet::Word => "\\w",
        StandardSet::NotWord => "\\W",
        StandardSet::Digit => "\\d",
        StandardSet::NotDigit => "\\D",
        // `.` is printed alone (see print_class_ranges); inside a class these never occur.
        StandardSet::NotLineTerminator | StandardSet::Everything => unreachable!(),
    }
}

fn property(p: Property, negated: bool) -> String {
    let body = match p {
        Property::GeneralCategory(v) => format!("gc={v}"),
        Property::Script(v) => format!("sc={v}"),
        Property::ScriptExtensions(v) => format!("scx={v}"),
        Property::Binary(v) | Property::OfStrings(v) => v.to_string(),
        Property::Any => "Any".to_string(),
        Property::Ascii => "ASCII".to_string(),
        Property::Assigned => "Assigned".to_string(),
    };
    format!("\\{}{{{body}}}", if negated { 'P' } else { 'p' })
}

fn push_item(out: &mut Vec<u32>, item: ClassItem, unicode: bool) {
    match item {
        ClassItem::Range(from, to) => {
            push_char(out, from, unicode);
            if to != from {
                out.push(u32::from('-'));
                push_char(out, to, unicode);
            }
        }
        ClassItem::Standard(s) => push_str(out, standard(s)),
        ClassItem::Property {
            property: p,
            negated,
        } => push_str(out, &property(p, negated)),
    }
}

/// The text of a non-`v` class, and the flags to compile it with (`.` sets `s` itself).
fn print_class_ranges(cr: &ep9_rx1::ClassRanges, f: Flags) -> (Vec<u32>, Flags) {
    let mut out = Vec::new();
    match cr.items.as_slice() {
        [ClassItem::Standard(StandardSet::NotLineTerminator)] if !cr.negated => {
            out.push(u32::from('.'));
            return (out, f.with(Flags::DOT_ALL, false));
        }
        [ClassItem::Standard(StandardSet::Everything)] if !cr.negated => {
            out.push(u32::from('.'));
            return (out, f.with(Flags::DOT_ALL, true));
        }
        // `\d \D \s \S \w \W` alone: the escape as the source has it.
        [ClassItem::Standard(set)] if !cr.negated => {
            push_str(&mut out, standard(*set));
            return (out, f.with(Flags::DOT_ALL, false));
        }
        [
            ClassItem::Property {
                property: p,
                negated,
            },
        ] if !cr.negated && f.either_unicode() => {
            push_str(&mut out, &property(*p, *negated));
            return (out, f.with(Flags::DOT_ALL, false));
        }
        _ => {}
    }
    out.push(u32::from('['));
    if cr.negated {
        out.push(u32::from('^'));
    }
    for &item in &cr.items {
        push_item(&mut out, item, f.either_unicode());
    }
    out.push(u32::from(']'));
    (out, f.with(Flags::DOT_ALL, false))
}

fn print_operand(out: &mut Vec<u32>, op: &ClassSetOperand) {
    out.push(u32::from('['));
    for &item in &op.items {
        push_item(out, item, true);
    }
    if !op.strings.is_empty() {
        push_str(out, "\\q{");
        for (i, key) in op.strings.keys().enumerate() {
            if i > 0 {
                out.push(u32::from('|'));
            }
            for &c in &key.0 {
                push_char(out, c, true);
            }
        }
        out.push(u32::from('}'));
    }
    out.push(u32::from(']'));
}

fn print_set(out: &mut Vec<u32>, t: &Tree) {
    match t {
        Tree::ClassSetOperand(op) => print_operand(out, op),
        Tree::ClassSetExpression(ClassSetExpression {
            operation,
            negated,
            operands,
            ..
        }) => {
            out.push(u32::from('['));
            if *negated {
                out.push(u32::from('^'));
            }
            for (i, o) in operands.iter().enumerate() {
                if i > 0 {
                    match operation {
                        ClassSetOperation::Union => {}
                        ClassSetOperation::Intersection => push_str(out, "&&"),
                        ClassSetOperation::Subtraction => push_str(out, "--"),
                    }
                }
                print_set(out, o);
            }
            out.push(u32::from(']'));
        }
        other => unreachable!("not a v-mode class: {other:?}"),
    }
}

// ---- the walk -------------------------------------------------------------------------------

#[derive(Default)]
struct Found {
    classes: Vec<Class>,
    /// (code unit, flags) of the atom characters under `i`.
    letters: BTreeSet<(u16, u8)>,
}

fn walk(t: &Tree, f: Flags, found: &mut Found) {
    match t {
        Tree::ClassRanges(cr) => {
            let (text, flags) = print_class_ranges(cr, f);
            found.classes.push(Class {
                text,
                flags: set_flags(flags),
                node: t.clone(),
            });
        }
        Tree::ClassSetOperand(_) | Tree::ClassSetExpression(_) => {
            let mut text = Vec::new();
            print_set(&mut text, t);
            found.classes.push(Class {
                text,
                flags: set_flags(f.with(Flags::DOT_ALL, false)),
                node: t.clone(),
            });
        }
        Tree::Atom(units) => {
            if f.ignore_case() {
                let flags = set_flags(f.with(Flags::DOT_ALL, false)).bits();
                let mut i = 0;
                while i < units.len() {
                    let u = units[i];
                    let pair = f.either_unicode()
                        && (0xd800..=0xdbff).contains(&u)
                        && units
                            .get(i + 1)
                            .is_some_and(|n| (0xdc00..=0xdfff).contains(n));
                    if pair {
                        // A supplementary character with no case equivalents (the parser
                        // makes the others classes).
                        i += 2;
                        continue;
                    }
                    found.letters.insert((u, flags));
                    i += 1;
                }
            }
        }
        Tree::Text(v) | Tree::Disjunction(v) | Tree::Alternative(v) => {
            for x in v {
                walk(x, f, found);
            }
        }
        Tree::Quantifier(q) => walk(&q.body, f, found),
        Tree::Capture(c) => walk(&c.body, f, found),
        Tree::Group(g) => walk(&g.body, g.flags, found),
        Tree::Lookaround(l) => walk(&l.body, f, found),
        Tree::Assertion(_) | Tree::BackReference(_) | Tree::Empty => {}
    }
}

// ---- RX0 ------------------------------------------------------------------------------------

fn rx0(text: &[u32], f: Flags, anchored: bool) -> Option<regress::Regex> {
    let rf = regress::Flags {
        icase: f.ignore_case(),
        multiline: false,
        dot_all: f.dot_all(),
        no_opt: false,
        unicode: f.either_unicode(),
        unicode_sets: f.unicode_sets(),
    };
    let mut chars = Vec::with_capacity(text.len() + 6);
    if anchored {
        chars.extend("^(?:".chars().map(u32::from));
    }
    chars.extend_from_slice(text);
    if anchored {
        chars.extend(")$".chars().map(u32::from));
    }
    regress::Regex::from_unicode(chars.into_iter(), rf).ok()
}

fn utf16(c: u32, out: &mut Vec<u16>) {
    if c > 0xffff {
        out.push((0xd800 + ((c - 0x10000) >> 10)) as u16);
        out.push((0xdc00 + ((c - 0x10000) & 0x3ff)) as u16);
    } else {
        out.push(c as u16);
    }
}

/// A subject that holds every code point (with `u`/`v`) or every code unit once, each one
/// character: the trail surrogates come before the lead surrogates, so no lead is right
/// before a trail.
struct Subject {
    units: Vec<u16>,
    /// The code point that starts at each unit index (`u32::MAX` inside a pair).
    at: Vec<u32>,
    unicode: bool,
}

fn subject(unicode: bool) -> Subject {
    let order: Vec<u32> = if unicode {
        (0..=0xd7ff)
            .chain(0xdc00..=0xdfff)
            .chain(0xd800..=0xdbff)
            .chain(0xe000..=0x10_ffff)
            .collect()
    } else {
        (0..=0xffff).collect()
    };
    let mut units = Vec::with_capacity(order.len() * 2);
    let mut at = Vec::with_capacity(order.len() * 2);
    for c in order {
        let start = units.len();
        utf16(c, &mut units);
        at.push(c);
        if units.len() - start == 2 {
            at.push(u32::MAX);
        }
    }
    Subject { units, at, unicode }
}

/// A member flag for each code point `0..=0x10FFFF`.
fn bitmap(ranges: &[ep9_rx1::CharacterRange], negated: bool) -> Vec<bool> {
    let mut v = vec![negated; 0x11_0000];
    for r in ranges {
        for c in r.from()..=r.to() {
            v[c as usize] = !negated;
        }
    }
    v
}

/// The characters regress matches in one search of the subject. `None` if a match is not
/// one character long.
fn rx0_members(re: &regress::Regex, s: &Subject) -> Option<Vec<bool>> {
    let found: Vec<std::ops::Range<usize>> = if s.unicode {
        re.find_from_utf16(&s.units, 0).map(|m| m.range).collect()
    } else {
        re.find_from_ucs2(&s.units, 0).map(|m| m.range).collect()
    };
    let mut out = vec![false; 0x11_0000];
    for r in found {
        let c = *s.at.get(r.start)?;
        if c == u32::MAX || r.end - r.start != if c > 0xffff { 2 } else { 1 } {
            return None;
        }
        out[c as usize] = true;
    }
    Some(out)
}

/// The member code points `0..=max` as `[from, to]` pairs.
fn member_ranges(m: &[bool], max: u32) -> Vec<[u32; 2]> {
    let mut out: Vec<[u32; 2]> = Vec::new();
    for c in 0..=max {
        if m[c as usize] {
            match out.last_mut() {
                Some(last) if last[1] + 1 == c => last[1] = c,
                _ => out.push([c, c]),
            }
        }
    }
    out
}

/// The first differences of two member maps over the subject's characters.
fn first_diffs(rx1: &[bool], rx0: &[bool], max: u32) -> Vec<String> {
    (0..=max)
        .filter(|&c| rx1[c as usize] != rx0[c as usize])
        .take(4)
        .map(|c| format!("U+{c:04X} rx1 {} rx0 {}", rx1[c as usize], rx0[c as usize]))
        .collect()
}

fn rx0_matches_string(re: &regress::Regex, f: Flags, s: &[u32]) -> bool {
    let mut units = Vec::new();
    for &c in s {
        utf16(c, &mut units);
    }
    let mut it = if f.either_unicode() {
        re.find_from_utf16(&units, 0)
            .map(|m| m.range)
            .collect::<Vec<_>>()
    } else {
        re.find_from_ucs2(&units, 0)
            .map(|m| m.range)
            .collect::<Vec<_>>()
    };
    it.retain(|r| r.start == 0 && r.end == units.len());
    !it.is_empty()
}

// ---- the known list -------------------------------------------------------------------------

/// `<class>\t<flags, - for none>\t<the class text, escaped>\t<the first differing probes>`;
/// `#` lines are comments.
fn known_list() -> BTreeMap<(String, String), String> {
    let text = include_str!("set_equality_known.txt");
    let mut m = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.splitn(4, '\t').collect();
        assert_eq!(f.len(), 4, "set_equality_known.txt:{}: 4 tab fields", n + 1);
        assert!(
            ["regress", "spec", "rx1-gap"].contains(&f[0]),
            "set_equality_known.txt:{}: class {}",
            n + 1,
            f[0]
        );
        let flags = if f[1] == "-" {
            String::new()
        } else {
            f[1].to_string()
        };
        let old = m.insert((f[2].to_string(), flags), f[0].to_string());
        assert!(
            old.is_none(),
            "set_equality_known.txt:{}: a second row",
            n + 1
        );
    }
    m
}

fn text16(text: &[u32]) -> Vec<u16> {
    let mut units = Vec::new();
    for &c in text {
        utf16(c, &mut units);
    }
    units
}

#[test]
#[ignore = "needs third_party/test262 (scripts/sync-test262.sh); run with --ignored"]
fn rx1_sets_match_rx0_on_the_test262_regexp_sources() {
    let (pairs, counts) = test262_pairs();
    let mut not_compared = BTreeMap::<&str, usize>::new();
    let mut classes: BTreeMap<(Vec<u32>, u8), Class> = BTreeMap::new();
    let mut letters: BTreeSet<(u16, u8)> = BTreeSet::new();
    let mut patterns = 0usize;
    for p in &pairs {
        let Some(flags) = Flags::parse(&p.flags) else {
            *not_compared.entry("invalid flags").or_default() += 1;
            continue;
        };
        let Ok(parsed) = ep9_rx1::parse(&p.pattern, flags) else {
            *not_compared.entry("RX1 refuses").or_default() += 1;
            continue;
        };
        if !corpus::rx0_accepts(p, flags) {
            *not_compared.entry("regress refuses").or_default() += 1;
            continue;
        }
        patterns += 1;
        let mut found = Found::default();
        walk(&parsed.tree, flags, &mut found);
        for c in found.classes {
            classes.entry((c.text.clone(), c.flags.bits())).or_insert(c);
        }
        letters.extend(found.letters);
    }

    // The probes of a class with strings: the fixed part.
    let mut base: BTreeSet<u32> = (0..=0x2ff).collect();
    base.extend((0..=0x10_ffffu32).step_by(61));
    base.insert(0x10_ffff);
    for c in 0..=0x10_ffffu32 {
        let f = simple_fold(c);
        if f != c {
            base.insert(c);
            base.insert(f);
        }
    }
    let subjects = [subject(false), subject(true)];

    let known = known_list();
    let mut dump = std::env::var_os("EP9_RX1_SETS").map(|_| String::new());
    let mut diffs: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    let mut probes_run = 0usize;
    let mut with_strings = 0usize;
    let mut strings_checked = 0usize;
    for ((text, bits), class) in &classes {
        let f = Flags::from_bits(*bits);
        let unicode = f.either_unicode();
        let max = if unicode { 0x10_ffff } else { 0xffff };
        let key = (escape(&text16(text)), f.to_letters());
        // RX1's code points and strings.
        let (rx1, strings): (Vec<bool>, Vec<Vec<u32>>) = match &class.node {
            Tree::ClassRanges(cr) => {
                let set = class_ranges(cr, f, false);
                (bitmap(&set.ranges, set.negated), Vec::new())
            }
            t => {
                let set = class_set(t, f).expect("a v-mode class");
                let strings = set
                    .strings
                    .keys()
                    .map(|k: &ClassString| k.0.clone())
                    .collect();
                (bitmap(&set.ranges, false), strings)
            }
        };
        if let Some(out) = dump.as_mut()
            && strings.is_empty()
        {
            let _ = writeln!(
                out,
                "{{\"t\":{:?},\"f\":\"{}\",\"r\":{:?}}}",
                text16(text),
                f.to_letters(),
                member_ranges(&rx1, max)
            );
        }
        let mut bad = Vec::new();
        if strings.is_empty() {
            let re = rx0(text, f, false)
                .unwrap_or_else(|| panic!("regress refuses the class text /{}/{}", key.0, key.1));
            probes_run += max as usize + 1;
            match rx0_members(&re, &subjects[usize::from(unicode)]) {
                Some(rx0) => bad = first_diffs(&rx1, &rx0, max),
                None => bad.push("a regress match is not one character".to_string()),
            }
        } else {
            with_strings += 1;
            let mut probes = base.clone();
            for (i, w) in rx1.windows(2).enumerate() {
                if w[0] != w[1] {
                    probes.extend([i as u32, i as u32 + 1]);
                }
            }
            probes.retain(|&c| c <= max);
            probes_run += probes.len();
            let re = rx0(text, f, true).expect("regress compiles the class text");
            for &c in &probes {
                let (a, b) = (rx1[c as usize], rx0_matches_string(&re, f, &[c]));
                if a != b && bad.len() < 4 {
                    bad.push(format!("U+{c:04X} rx1 {a} rx0 {b}"));
                }
            }
            let set: BTreeSet<&Vec<u32>> = strings.iter().collect();
            let fold = |s: &[u32]| -> Vec<u32> {
                if f.ignore_case() {
                    s.iter().map(|&c| simple_fold(c)).collect()
                } else {
                    s.to_vec()
                }
            };
            for s in &strings {
                let mut tries = vec![s.clone()];
                if s.len() > 1 {
                    tries.push(s[..s.len() - 1].to_vec());
                }
                let mut longer = s.clone();
                longer.push(u32::from('x'));
                tries.push(longer);
                for t in tries {
                    strings_checked += 1;
                    let a = if t.len() == 1 {
                        rx1[t[0] as usize]
                    } else {
                        set.contains(&fold(&t))
                    };
                    let b = rx0_matches_string(&re, f, &t);
                    if a != b && bad.len() < 4 {
                        bad.push(format!("{t:X?} rx1 {a} rx0 {b}"));
                    }
                }
            }
        }
        if !bad.is_empty() {
            diffs.insert(key, bad);
        }
    }

    // The atom letters under `i`, on every code unit.
    let mut letters_compared = 0usize;
    for &(u, bits) in &letters {
        let f = Flags::from_bits(bits);
        let unicode = f.either_unicode();
        let mut text = Vec::new();
        push_char(&mut text, u32::from(u), unicode);
        let key = (escape(&text16(&text)), f.to_letters());
        let mut rx1 = vec![false; 0x11_0000];
        for l in case_independent_letters(u, unicode, false) {
            rx1[usize::from(l)] = true;
        }
        let re = rx0(&text, f, false).expect("regress compiles one character");
        letters_compared += 1;
        probes_run += 0x1_0000;
        let Some(rx0) = rx0_members(&re, &subjects[usize::from(unicode)]) else {
            diffs.insert(
                key,
                vec!["a regress match is not one character".to_string()],
            );
            continue;
        };
        // A supplementary equivalent cannot be a letter of a BMP atom (the parser makes such
        // characters classes), so only the code units are compared.
        let bad = first_diffs(&rx1, &rx0, 0xffff);
        if !bad.is_empty() {
            diffs.insert(key, bad);
        }
    }

    if let (Some(path), Some(out)) = (std::env::var_os("EP9_RX1_SETS"), dump) {
        std::fs::write(path, out).unwrap();
    }
    let mut report = Vec::new();
    let mut by_class = BTreeMap::<String, usize>::new();
    for (key, bad) in &diffs {
        match known.get(key) {
            Some(class) => *by_class.entry(class.clone()).or_default() += 1,
            None => report.push(format!(
                "new:\t{}\t{}\t{}",
                if key.1.is_empty() { "-" } else { &key.1 },
                key.0,
                bad.join("; ")
            )),
        }
    }
    for (key, class) in &known {
        if !diffs.contains_key(key) {
            report.push(format!("gone: {class}\t{}\t{}", key.1, key.0));
        }
    }
    let mut summary = String::new();
    let _ = write!(
        summary,
        "set-equality: {} files, {} unique pairs, {} patterns compared, not compared {:?}; \
         {} unique classes ({} with strings, {} string checks), {} atom letters, {} probes; \
         {} differ {:?}",
        counts.files,
        pairs.len(),
        patterns,
        not_compared,
        classes.len(),
        with_strings,
        strings_checked,
        letters_compared,
        probes_run,
        diffs.len(),
        by_class
    );
    println!("{summary}");
    for line in &report {
        println!("{line}");
    }
    // Floors (M7-P5 counts, rounded down).
    assert!(patterns >= 4500, "only {patterns} patterns");
    assert!(classes.len() >= 1100, "only {} classes", classes.len());
    assert!(
        with_strings >= 35,
        "only {with_strings} classes with strings"
    );
    assert!(
        letters_compared >= 30,
        "only {letters_compared} atom letters"
    );
    assert!(
        report.is_empty(),
        "the differences are not the known list (tests/set_equality_known.txt):\n{}",
        report.join("\n")
    );
}
