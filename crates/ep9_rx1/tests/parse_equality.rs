//! The parse-equality test: RX1 against RX0 on the RegExp sources of test262 (plan v12
//! section 7.2; decisions-log M6-ORCH-300).
//!
//! ```text
//! third_party/test262/test/built-ins/RegExp/**/*.js      regexp literals + RegExp("..", "..")
//! third_party/test262/test/language/literals/regexp/**    regexp literals
//!   -> unique (pattern, flags) pairs
//!   -> flags: Flags::parse (an invalid flags string is counted, not compared)
//!   -> RX1: ep9_rx1::parse            RX0: regress::Regex::from_unicode
//!   -> the pairs where one accepts and the other refuses == tests/parse_equality_known.txt
//! ```
//!
//! RX0 here is regress (the fork of the root `[patch.crates-io]`), called as `ep9_regexp`
//! calls it but without its pre-pass: `ep9_regexp` rewrites some escapes before regress sees
//! the pattern (`legacy_chars`, `unicode_chars`), and this crate may not depend on
//! `ep9_regexp` (dep-guards check 14). The patterns that the pre-pass changes are named in the
//! known list as `prepass`. Every other known difference is named too; a new difference, or a
//! listed one that is gone, fails the test.
//!
//! What it trusts: the extractor below reads JavaScript with a small lexer (comments, strings,
//! templates, the regexp-or-divide rule by the previous token). A constructor call whose
//! arguments are not string literals is counted and skipped. The test fails if the counts
//! fall under the floors in [`check_floors`], so a broken extractor cannot pass with few pairs.
//!
//! Run: `bash scripts/sync-test262.sh`, then
//! `cargo test --locked -p ep9_rx1 --test parse_equality -- --ignored --nocapture`.
//! Set `EP9_RX1_PAIRS=<file>` to write every compared pair (JSON lines) for a check against
//! node.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use ep9_rx1::Flags;

/// One RegExp source: the pattern and the flags text, as UTF-16 code units.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Pair {
    pattern: Vec<u16>,
    flags: Vec<u16>,
}

#[derive(Default, Debug)]
struct Counts {
    files: usize,
    literals: usize,
    ctor_calls: usize,
    ctor_skipped: usize,
}

// ---- the JavaScript extractor ---------------------------------------------------------------

/// True if a `/` after this token starts a regexp literal (not a division).
fn regexp_allowed(prev: &Prev) -> bool {
    match prev {
        Prev::None => true,
        Prev::Punct(c) => !matches!(c, ')' | ']'),
        Prev::Word(w) => matches!(
            w.as_str(),
            "return"
                | "typeof"
                | "instanceof"
                | "in"
                | "of"
                | "new"
                | "delete"
                | "void"
                | "throw"
                | "case"
                | "do"
                | "else"
                | "yield"
                | "await"
        ),
        Prev::Value => false,
    }
}

#[derive(Clone, Debug)]
enum Prev {
    None,
    Punct(char),
    Word(String),
    /// A number, string, template or regexp literal.
    Value,
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn is_ident_part(c: char) -> bool {
    c.is_alphanumeric() || c == '$' || c == '_' || c == '\\' || c == '\u{200c}' || c == '\u{200d}'
}

/// A string literal at `chars[i]` (a quote): its UTF-16 value and the index after it, or
/// `None` if it does not end on its line.
fn string_literal(chars: &[char], mut i: usize) -> Option<(Vec<u16>, usize)> {
    let quote = chars[i];
    i += 1;
    let mut out: Vec<u16> = Vec::new();
    let hex = |s: &[char]| -> Option<u32> {
        let t: String = s.iter().collect();
        u32::from_str_radix(&t, 16).ok()
    };
    let push = |out: &mut Vec<u16>, cp: u32| {
        if cp > 0xffff {
            let c = cp - 0x10000;
            out.push(0xd800 + (c >> 10) as u16);
            out.push(0xdc00 + (c & 0x3ff) as u16);
        } else {
            out.push(cp as u16);
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if c == quote {
            return Some((out, i + 1));
        }
        if c == '\n' || c == '\r' {
            return None;
        }
        if c != '\\' {
            let mut b = [0u16; 2];
            out.extend_from_slice(c.encode_utf16(&mut b));
            i += 1;
            continue;
        }
        let d = *chars.get(i + 1)?;
        i += 2;
        match d {
            'n' => out.push(0x0a),
            't' => out.push(0x09),
            'r' => out.push(0x0d),
            'b' => out.push(0x08),
            'f' => out.push(0x0c),
            'v' => out.push(0x0b),
            '0' if !chars.get(i).is_some_and(|c| c.is_ascii_digit()) => out.push(0),
            'x' => {
                let v = hex(chars.get(i..i + 2)?)?;
                out.push(v as u16);
                i += 2;
            }
            'u' if chars.get(i) == Some(&'{') => {
                let end = i + chars[i..].iter().position(|&c| c == '}')?;
                push(&mut out, hex(&chars[i + 1..end])?);
                i = end + 1;
            }
            'u' => {
                let v = hex(chars.get(i..i + 4)?)?;
                out.push(v as u16);
                i += 4;
            }
            '\r' => {
                if chars.get(i) == Some(&'\n') {
                    i += 1;
                }
            }
            '\n' | '\u{2028}' | '\u{2029}' => {}
            // Legacy octal escapes (sloppy code): not in the RegExp tests' string arguments;
            // a call with one is skipped by the caller.
            '1'..='9' | '0' => return None,
            other => push(&mut out, u32::from(other)),
        }
    }
    None
}

/// The RegExp sources of one file: regexp literals, and (with `ctors`) the string literal
/// arguments of `RegExp(` calls.
fn extract(text: &str, ctors: bool, counts: &mut Counts, out: &mut Vec<Pair>) {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut prev = Prev::None;
    // Template literal nesting: each entry is the brace depth at which a `${` opened.
    let mut templates: Vec<usize> = Vec::new();
    let mut braces = 0usize;
    while i < n {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < n && !is_line_terminator(chars[i]) {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < n && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == '\'' || c == '"' {
            i = match string_literal(&chars, i) {
                Some((_, end)) => end,
                None => i + 1,
            };
            prev = Prev::Value;
            continue;
        }
        if c == '`' || (c == '}' && templates.last() == Some(&braces)) {
            // A template part: up to the closing backquote or the next `${`.
            if c == '}' {
                templates.pop();
            }
            i += 1;
            prev = Prev::Value;
            while i < n {
                if chars[i] == '\\' {
                    i += 2;
                } else if chars[i] == '`' {
                    i += 1;
                    break;
                } else if chars[i] == '$' && chars.get(i + 1) == Some(&'{') {
                    // A substitution: code follows, as after `{`.
                    i += 2;
                    templates.push(braces);
                    prev = Prev::Punct('{');
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if c == '/'
            && regexp_allowed(&prev)
            && let Some((pattern, flags, end)) = regexp_literal(&chars, i)
        {
            counts.literals += 1;
            out.push(Pair { pattern, flags });
            i = end;
            prev = Prev::Value;
            continue;
        }
        if is_ident_part(c) && c != '\\' {
            let start = i;
            while i < n && is_ident_part(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if c.is_ascii_digit() {
                prev = Prev::Value;
                continue;
            }
            if ctors && word == "RegExp" {
                regexp_call(&chars, i, counts, out);
            }
            prev = Prev::Word(word);
            continue;
        }
        if c == '{' {
            braces += 1;
        } else if c == '}' {
            braces = braces.saturating_sub(1);
        }
        prev = Prev::Punct(c);
        i += 1;
    }
}

/// A regexp literal at `chars[i]` = `/`: the body and the flags (UTF-16) and the index after
/// it, by the lexical grammar (RegularExpressionLiteral, ECMA-262 12.9.5).
fn regexp_literal(chars: &[char], mut i: usize) -> Option<(Vec<u16>, Vec<u16>, usize)> {
    i += 1;
    let start = i;
    let mut in_class = false;
    loop {
        let c = *chars.get(i)?;
        if is_line_terminator(c) {
            return None;
        }
        match c {
            '\\' => {
                let d = *chars.get(i + 1)?;
                if is_line_terminator(d) {
                    return None;
                }
                i += 2;
                continue;
            }
            '[' => in_class = true,
            ']' => in_class = false,
            '/' if !in_class => break,
            _ => {}
        }
        i += 1;
    }
    if i == start || chars[start] == '*' {
        return None;
    }
    let body: String = chars[start..i].iter().collect();
    i += 1;
    let fstart = i;
    while i < chars.len() && is_ident_part(chars[i]) {
        i += 1;
    }
    let flags: String = chars[fstart..i].iter().collect();
    Some((
        body.encode_utf16().collect(),
        flags.encode_utf16().collect(),
        i,
    ))
}

/// `RegExp(` after the word at `chars[i]`: the pattern and flags string literals, if both are
/// plain string literals (else counted as skipped).
fn regexp_call(chars: &[char], mut i: usize, counts: &mut Counts, out: &mut Vec<Pair>) {
    let skip_ws = |chars: &[char], mut i: usize| {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        i
    };
    i = skip_ws(chars, i);
    if chars.get(i) != Some(&'(') {
        return;
    }
    counts.ctor_calls += 1;
    i = skip_ws(chars, i + 1);
    let lit = |i: usize| match chars.get(i) {
        Some('\'' | '"') => string_literal(chars, i),
        _ => None,
    };
    let Some((pattern, end)) = lit(i) else {
        counts.ctor_skipped += 1;
        return;
    };
    i = skip_ws(chars, end);
    let flags = match chars.get(i) {
        Some(')') => Vec::new(),
        Some(',') => {
            let j = skip_ws(chars, i + 1);
            match lit(j) {
                Some((f, end)) if chars.get(skip_ws(chars, end)) == Some(&')') => f,
                _ => {
                    counts.ctor_skipped += 1;
                    return;
                }
            }
        }
        _ => {
            counts.ctor_skipped += 1;
            return;
        }
    };
    out.push(Pair { pattern, flags });
}

fn js_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            js_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "js") {
            out.push(p);
        }
    }
}

// ---- the two parsers ------------------------------------------------------------------------

/// RX0's engine as `ep9_regexp` calls it (the flags mapping of `Rx0::compile`), without the
/// pre-pass: in Unicode mode surrogate pairs are joined, else one character per code unit.
fn rx0_accepts(p: &Pair, flags: Flags) -> bool {
    let unicode = flags.either_unicode();
    let rf = regress::Flags {
        icase: flags.ignore_case(),
        multiline: flags.multiline(),
        dot_all: flags.dot_all(),
        no_opt: false,
        unicode,
        unicode_sets: flags.unicode_sets(),
    };
    let chars: Vec<u32> = if unicode {
        char::decode_utf16(p.pattern.iter().copied())
            .map(|r| match r {
                Ok(c) => u32::from(c),
                Err(e) => u32::from(e.unpaired_surrogate()),
            })
            .collect()
    } else {
        p.pattern.iter().map(|&u| u32::from(u)).collect()
    };
    regress::Regex::from_unicode(chars.into_iter(), rf).is_ok()
}

// ---- the known list -------------------------------------------------------------------------

/// The known-difference file form: `<class>\t<flags>\t<rx1: ok|err>\t<pattern>`, the pattern
/// escaped by [`escape`]. `#` lines are comments.
fn escape(units: &[u16]) -> String {
    let mut s = String::new();
    for &u in units {
        match u {
            0x5c => s.push_str("\\\\"),
            0x20..=0x7e => s.push(char::from(u as u8)),
            _ => {
                let _ = write!(s, "\\u{u:04X}");
            }
        }
    }
    s
}

fn known_list() -> BTreeMap<(String, String), (String, String)> {
    let text = include_str!("parse_equality_known.txt");
    let mut m = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.splitn(4, '\t').collect();
        assert_eq!(
            f.len(),
            4,
            "parse_equality_known.txt:{}: 4 tab fields",
            n + 1
        );
        assert!(
            ["prepass", "regress", "rx1-gap", "spec"].contains(&f[0]),
            "parse_equality_known.txt:{}: class {}",
            n + 1,
            f[0]
        );
        assert!(
            ["ok", "err"].contains(&f[2]),
            "parse_equality_known.txt:{}",
            n + 1
        );
        let flags = if f[1] == "-" {
            String::new()
        } else {
            f[1].to_string()
        };
        let old = m.insert(
            (f[3].to_string(), flags),
            (f[0].to_string(), f[2].to_string()),
        );
        assert!(
            old.is_none(),
            "parse_equality_known.txt:{}: a second row",
            n + 1
        );
    }
    m
}

/// The floors of the extractor (M7-P4 counts, rounded down): fewer means a broken extractor
/// or a changed corpus, and the test fails rather than pass on less.
fn check_floors(counts: &Counts, compared: usize) {
    assert!(counts.files >= 2100, "only {} files", counts.files);
    assert!(counts.literals >= 5400, "only {} literals", counts.literals);
    assert!(
        counts.ctor_calls >= 600,
        "only {} RegExp calls",
        counts.ctor_calls
    );
    assert!(compared >= 5000, "only {compared} unique pairs compared");
}

#[test]
#[ignore = "needs third_party/test262 (scripts/sync-test262.sh); run with --ignored"]
fn rx1_parses_the_test262_regexp_sources_as_rx0() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../third_party/test262/test");
    assert!(
        root.join("built-ins/RegExp").is_dir(),
        "{} is missing: run scripts/sync-test262.sh (this test never skips)",
        root.display()
    );
    let mut counts = Counts::default();
    let mut pairs = Vec::new();
    for (dir, ctors) in [
        ("built-ins/RegExp", true),
        ("language/literals/regexp", false),
    ] {
        let mut files = Vec::new();
        js_files(&root.join(dir), &mut files);
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            counts.files += 1;
            extract(&text, ctors, &mut counts, &mut pairs);
        }
    }
    let unique: BTreeSet<Pair> = pairs.into_iter().collect();
    let mut bad_flags = 0;
    let mut compared = 0;
    let mut diffs: BTreeMap<(String, String), bool> = BTreeMap::new();
    let mut dump = String::new();
    for p in &unique {
        let Some(flags) = Flags::parse(&p.flags) else {
            bad_flags += 1;
            continue;
        };
        compared += 1;
        let parsed = ep9_rx1::parse(&p.pattern, flags);
        let rx1 = parsed.is_ok();
        let rx0 = rx0_accepts(p, flags);
        if rx1 != rx0 {
            diffs.insert((escape(&p.pattern), flags.to_letters()), rx1);
        }
        // For the node check: the group count and the group names (sorted, unique).
        let (groups, names) = match &parsed {
            Ok(pat) => {
                let mut names: Vec<String> = pat
                    .named_captures
                    .iter()
                    .map(|(n, _)| format!("{n:?}"))
                    .collect();
                names.dedup();
                (pat.capture_count, names.join(","))
            }
            Err(_) => (0, String::new()),
        };
        let _ = writeln!(
            dump,
            "{{\"p\":{:?},\"f\":\"{}\",\"rx1\":{rx1},\"rx0\":{rx0},\"groups\":{groups},\"names\":[{names}]}}",
            p.pattern,
            flags.to_letters()
        );
    }
    if let Ok(path) = std::env::var("EP9_RX1_PAIRS") {
        std::fs::write(path, dump).unwrap();
    }
    let known = known_list();
    let mut by_class: BTreeMap<String, usize> = BTreeMap::new();
    let mut report = Vec::new();
    for (key, &rx1) in &diffs {
        match known.get(key) {
            Some((class, verdict)) if (verdict == "ok") == rx1 => {
                *by_class.entry(class.clone()).or_default() += 1;
            }
            Some((class, verdict)) => report.push(format!(
                "changed: {class}\t{}\t{verdict} -> {}\t{}",
                key.1,
                if rx1 { "ok" } else { "err" },
                key.0
            )),
            None => report.push(format!(
                "new:\t{}\t{}\t{}",
                if key.1.is_empty() { "-" } else { &key.1 },
                if rx1 { "ok" } else { "err" },
                key.0
            )),
        }
    }
    for (key, (class, _)) in &known {
        if !diffs.contains_key(key) {
            report.push(format!("gone: {class}\t{}\t{}", key.1, key.0));
        }
    }
    println!(
        "parse-equality: {} files, {} literals, {} RegExp calls ({} skipped: not string \
         literals), {} unique pairs, {} with invalid flags, {} compared, {} differ {:?}",
        counts.files,
        counts.literals,
        counts.ctor_calls,
        counts.ctor_skipped,
        unique.len(),
        bad_flags,
        compared,
        diffs.len(),
        by_class
    );
    check_floors(&counts, compared);
    assert!(
        report.is_empty(),
        "the differences are not the known list (tests/parse_equality_known.txt):\n{}",
        report.join("\n")
    );
}

#[test]
fn the_extractor_reads_literals_calls_and_skips_the_rest() {
    let src = r#"
        /*--- a: /not/ ---*/
        var a = /x\/y[/]z/gi; // /comment/
        var b = c / d / e;
        var s = "/not/" + '\'/';
        var t = `/not/ ${ /in-template/u } /not/`;
        new RegExp("\\u0041b", "v"); RegExp('c'); RegExp(x); RegExp("d", f);
        if (x) /after-paren/; return /ret/y;
    "#;
    let mut counts = Counts::default();
    let mut out = Vec::new();
    extract(src, true, &mut counts, &mut out);
    let got: Vec<(String, String)> = out
        .iter()
        .map(|p| {
            (
                String::from_utf16(&p.pattern).unwrap(),
                String::from_utf16(&p.flags).unwrap(),
            )
        })
        .collect();
    let want: Vec<(String, String)> = [
        ("x\\/y[/]z", "gi"),
        ("in-template", "u"),
        ("\\u0041b", "v"),
        ("c", ""),
        ("ret", "y"),
    ]
    .iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    // `/after-paren/` after `)` is read as a division (the lexer cannot tell `if (x)` from
    // `(x)`); test262 RegExp sources do not use that form.
    assert_eq!(got, want);
    assert_eq!(
        (counts.literals, counts.ctor_calls, counts.ctor_skipped),
        (3, 4, 2)
    );
}
