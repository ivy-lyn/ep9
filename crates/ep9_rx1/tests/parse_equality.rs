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
//!
//! The same pairs also check port step 3: each pattern RX1 parses compiles in both forms and
//! its graph is valid ([`rx1_compiles_the_test262_regexp_sources`]).

mod corpus;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use corpus::{Counts, escape, extract, rx0_accepts, test262_pairs};
use ep9_rx1::Flags;

// ---- the known list -------------------------------------------------------------------------

/// The known-difference file form: `<class>\t<flags>\t<rx1: ok|err>\t<pattern>`, the pattern
/// escaped by [`escape`]. `#` lines are comments.
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
    let (unique, counts) = test262_pairs();
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

/// Port step 3 on the same pairs: every pattern RX1 parses compiles to a node graph in both
/// forms (one-byte and two-byte) and passes [`ep9_rx1::Program::validate`]. The graph is
/// built and analysed on a thread with a large stack, as the engine will run it.
#[test]
#[ignore = "needs third_party/test262 (scripts/sync-test262.sh); run with --ignored"]
fn rx1_compiles_the_test262_regexp_sources() {
    let (unique, _) = test262_pairs();
    let report = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            let mut compiled = 0;
            let mut report = Vec::new();
            for p in &unique {
                let Some(flags) = Flags::parse(&p.flags) else {
                    continue;
                };
                let Ok(pattern) = ep9_rx1::parse(&p.pattern, flags) else {
                    continue;
                };
                for one_byte in [false, true] {
                    let options = ep9_rx1::CompileOptions::for_source(one_byte, p.pattern.len());
                    match ep9_rx1::compile(&pattern, flags, options) {
                        Ok(program) => match program.validate() {
                            Ok(()) => compiled += 1,
                            Err(e) => report.push(format!(
                                "{}\t{}\t{one_byte}\t{e}",
                                flags.to_letters(),
                                escape(&p.pattern)
                            )),
                        },
                        Err(e) => report.push(format!(
                            "{}\t{}\t{one_byte}\t{e:?}",
                            flags.to_letters(),
                            escape(&p.pattern)
                        )),
                    }
                }
            }
            println!("compile: {compiled} programs, {} errors", report.len());
            assert!(compiled >= 8500, "only {compiled} programs");
            report
        })
        .expect("spawn")
        .join()
        .expect("the compile thread");
    assert!(report.is_empty(), "compile errors:\n{}", report.join("\n"));
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
