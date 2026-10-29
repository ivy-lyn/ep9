//! The RX0A differential golden (plan v9 §4.5): `tests/golden/node-pairs.txt` holds 535
//! (pattern, flags, lastIndex, input) cases with the result of one `RegExp.prototype.exec` in
//! node v24.22.1 (its header names the format): 500 picked cases, then 23 added by the M4-P2
//! Codex-fix step (surrogate escapes in `u` and `v`, non-Unicode ignore case), then 12 by
//! Codex r2 (`\k` with and without a group name, no `u`). This test runs each case through
//! [`Rx0`] and prints every line that differs.
//!
//! ```text
//! line     pattern \t flags \t lastIndex \t input \t result
//! start    lastIndex if the flags have g or y, else 0 (RegExpBuiltinExec, §22.2.7.2)
//! input    Input::Latin1 when every code unit is below 0x100 (the VM's one-byte form),
//!          else Input::Utf16
//! result   null | error | match s-e ... ("-" = a capture that did not take part)
//! ```
//!
//! [`KNOWN`] lists the lines where RX0 differs from node, with the RX0 result. It held 11
//! lines of one regress 0.12.0 defect (non-Unicode `i` without the ASCII rule of Canonicalize,
//! §22.2.2.7.3) until the fork patch 0004 (M5-P3 RX0b, `third_party/patches/regress/`); it is
//! now empty and must stay so (P2-ORCH-1). The test fails if any line differs from node.

use ep9_regexp::{Backend, ExecResult, Flags, Input, Rx0, Rx0Scratch};

/// Reads a golden string: `\\` is a backslash, `\uXXXX` any code unit, the rest as is.
fn unescape(s: &str) -> Vec<u16> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && b.get(i + 1) == Some(&b'\\') {
            out.push(u16::from(b'\\'));
            i += 2;
        } else if b[i] == b'\\' && b.get(i + 1) == Some(&b'u') {
            let hex = std::str::from_utf8(&b[i + 2..i + 6]).unwrap();
            out.push(u16::from_str_radix(hex, 16).unwrap());
            i += 6;
        } else {
            out.push(u16::from(b[i]));
            i += 1;
        }
    }
    out
}

/// The result text of one case, in the golden's form.
fn run(pattern: &[u16], flags: &str, last_index: usize, input: &[u16]) -> String {
    let flags16: Vec<u16> = flags.encode_utf16().collect();
    let flags = Flags::parse(&flags16).expect("the golden has valid flag letters");
    let program = match Rx0::compile(pattern, flags) {
        Ok(p) => p,
        Err(_) => return "error".into(),
    };
    let start = if flags.contains(Flags::GLOBAL) || flags.contains(Flags::STICKY) {
        last_index
    } else {
        0
    };
    let latin1: Option<Vec<u8>> = input.iter().map(|&u| u8::try_from(u).ok()).collect();
    let input = match &latin1 {
        Some(bytes) => Input::Latin1(bytes),
        None => Input::Utf16(input),
    };
    let mut caps = Vec::new();
    let mut scratch = Rx0Scratch::default();
    match Rx0::exec(&program, input, start, &mut caps, &mut scratch, &mut || {
        false
    }) {
        ExecResult::NoMatch => "null".into(),
        ExecResult::Match => {
            let spans: Vec<String> = caps
                .iter()
                .map(|c| match c {
                    Some((s, e)) => format!("{s}-{e}"),
                    None => "-".into(),
                })
                .collect();
            format!("match {}", spans.join(" "))
        }
        r => format!("{r:?}"),
    }
}

/// Golden lines that differ from node, as (the golden line, the RX0 result): none since the
/// fork patch 0004.
const KNOWN: &[(&str, &str)] = &[];

#[test]
fn rx0_matches_node() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/node-pairs.txt");
    let text = std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
    let mut cases = 0;
    let mut diffs = Vec::new();
    let mut known = 0;
    for (n, line) in text.lines().enumerate() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols.len(), 5, "line {}: 5 columns", n + 1);
        cases += 1;
        let got = run(
            &unescape(cols[0]),
            cols[1],
            cols[2].parse().unwrap(),
            &unescape(cols[3]),
        );
        let listed = KNOWN.iter().find(|(l, _)| *l == line).map(|(_, r)| *r);
        match listed {
            Some(r) if r == got => known += 1,
            Some(r) => diffs.push(format!(
                "line {}: {line}\n    rx0: {got} (KNOWN lists {r}; update the list)",
                n + 1
            )),
            None if got != cols[4] => {
                diffs.push(format!("line {}: {line}\n    rx0: {got}", n + 1));
            }
            None => {}
        }
    }
    assert_eq!(cases, 535, "the golden has 535 cases");
    assert_eq!(known, KNOWN.len(), "every KNOWN line is in the golden");
    println!(
        "{} of {cases} equal node, {known} known regress differences",
        cases - known
    );
    assert!(
        diffs.is_empty(),
        "{} of {cases} cases differ from node:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}
