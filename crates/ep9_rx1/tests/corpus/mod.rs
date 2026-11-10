//! The test262 RegExp corpus of the RX0 diff tests (tests/parse_equality.rs,
//! tests/set_equality.rs): a small JavaScript extractor, the file walk and RX0's engine as
//! `ep9_regexp` calls it. Each test binary uses a part of it.
#![allow(dead_code)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use ep9_rx1::Flags;

/// One RegExp source: the pattern and the flags text, as UTF-16 code units.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Pair {
    pub(crate) pattern: Vec<u16>,
    pub(crate) flags: Vec<u16>,
}

#[derive(Default, Debug)]
pub(crate) struct Counts {
    pub(crate) files: usize,
    pub(crate) literals: usize,
    pub(crate) ctor_calls: usize,
    pub(crate) ctor_skipped: usize,
}

// ---- the JavaScript extractor ---------------------------------------------------------------

/// True if a `/` after this token starts a regexp literal (not a division).
pub(crate) fn regexp_allowed(prev: &Prev) -> bool {
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
pub(crate) enum Prev {
    None,
    Punct(char),
    Word(String),
    /// A number, string, template or regexp literal.
    Value,
}

pub(crate) fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

pub(crate) fn is_ident_part(c: char) -> bool {
    c.is_alphanumeric() || c == '$' || c == '_' || c == '\\' || c == '\u{200c}' || c == '\u{200d}'
}

/// A string literal at `chars[i]` (a quote): its UTF-16 value and the index after it, or
/// `None` if it does not end on its line.
pub(crate) fn string_literal(chars: &[char], mut i: usize) -> Option<(Vec<u16>, usize)> {
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
pub(crate) fn extract(text: &str, ctors: bool, counts: &mut Counts, out: &mut Vec<Pair>) {
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
pub(crate) fn regexp_literal(chars: &[char], mut i: usize) -> Option<(Vec<u16>, Vec<u16>, usize)> {
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
pub(crate) fn regexp_call(chars: &[char], mut i: usize, counts: &mut Counts, out: &mut Vec<Pair>) {
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

pub(crate) fn js_files(dir: &Path, out: &mut Vec<PathBuf>) {
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

// ---- RX0 ------------------------------------------------------------------------------------

/// RX0's engine as `ep9_regexp` calls it (the flags mapping of `Rx0::compile`), without the
/// pre-pass: in Unicode mode surrogate pairs are joined, else one character per code unit.
pub(crate) fn rx0_accepts(p: &Pair, flags: Flags) -> bool {
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

/// A pattern for a file of known differences: a backslash as `\\`, every code unit outside
/// ASCII 0x20-0x7E as `\uXXXX`.
pub(crate) fn escape(units: &[u16]) -> String {
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

/// The unique (pattern, flags) pairs of `built-ins/RegExp` (literals and `RegExp(..)` calls)
/// and `language/literals/regexp` (literals) of the test262 checkout, with the counts. Panics
/// if the checkout is missing (the tests that call it never skip).
pub(crate) fn test262_pairs() -> (std::collections::BTreeSet<Pair>, Counts) {
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
    (pairs.into_iter().collect(), counts)
}
