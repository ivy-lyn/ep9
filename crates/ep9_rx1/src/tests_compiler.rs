//! Unit tests of port step 3 (the compiler: the node graph and the analysis).
//!
//! ```text
//! tests/compiler_exec.txt (node v26 results)
//!   -> parse, compile two-byte (and one-byte for a Latin-1 subject), Program::validate
//!   -> the reference walker (compiler/walker.rs) runs the graph   == node's match
//!   -> on each successful path: eats-at-least and the quick checks hold
//!   -> on the match: the search-loop Boyer-Moore map and the one-byte filters hold
//! ```
//!
//! `EP9_RX1_EXEC=<file>` runs the same check on a larger file of the same form (the
//! test262 corpus rows; see [`compiled_programs_match_node_on_a_corpus_file`]).

use super::*;
use crate::compiler::walker::{Groups, Walker};

fn flags(s: &str) -> Flags {
    let units: Vec<u16> = s.encode_utf16().collect();
    Flags::parse(&units).expect("valid flags")
}

/// The step budget of one walk.
const STEPS: u64 = 2_000_000;

/// The stack of the thread that walks (the walker recurses once per node visit).
const WALK_STACK: usize = 512 << 20;

/// Reads the escape form of the fixture: `\\` and `\uXXXX`.
fn unescape(s: &str) -> Vec<u16> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && b.get(i + 1) == Some(&b'\\') {
            out.push(0x5c);
            i += 2;
        } else if b[i] == b'\\' && b.get(i + 1) == Some(&b'u') {
            out.push(u16::from_str_radix(&s[i + 2..i + 6], 16).expect("\\uXXXX"));
            i += 6;
        } else {
            out.push(u16::from(b[i]));
            i += 1;
        }
    }
    out
}

fn parse_result(s: &str) -> Groups {
    if s == "null" {
        return None;
    }
    Some(
        s.split(';')
            .map(|g| {
                (g != "-").then(|| {
                    let (a, b) = g.split_once(',').expect("start,end");
                    (a.parse().expect("start"), b.parse().expect("end"))
                })
            })
            .collect(),
    )
}

struct Row {
    flags: String,
    pattern: Vec<u16>,
    subject: Vec<u16>,
    expected: Groups,
}

fn rows(text: &str) -> Vec<Row> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            assert_eq!(f.len(), 4, "4 tab fields: {l}");
            Row {
                flags: if f[0] == "-" {
                    String::new()
                } else {
                    f[0].to_owned()
                },
                pattern: unescape(f[1]),
                subject: unescape(f[2]),
                expected: parse_result(f[3]),
            }
        })
        .collect()
}

/// The facts the match of `program` on `subject` must keep: the Boyer-Moore map of the
/// search loop, and the one-byte filters of a pattern with no search loop.
fn check_match(program: &Program, subject: &[u16], groups: &Groups, problems: &mut Vec<String>) {
    let Some(groups) = groups else {
        return;
    };
    let (s, _) = groups[0].expect("group 0");
    if let Some(bm) = program
        .reachable()
        .into_iter()
        .find_map(|id| program.search_loop_lookahead(id))
    {
        for i in 0..bm.length() {
            if let Some(&c) = subject.get(s + i)
                && !bm.at(i).at(usize::from(c) & 127)
            {
                problems.push(format!("Boyer-Moore position {i} has no {c:#x}"));
            }
        }
    }
    if program.one_byte() && !program.has_search_prefix() {
        let filters = program.quick_check_filters();
        if let (Some(&c), Some(reject)) = (subject.first(), filters.reject_bitset)
            && reject[usize::from(c) / 32] >> (c % 32) & 1 != 0
        {
            problems.push(format!("the reject bitset rejects {c:#x}"));
        }
        if let Some((mask, value)) = filters.mask_value {
            let n = program.eats_at_least(program.start(), false).min(4) as usize;
            let word = subject[..n]
                .iter()
                .enumerate()
                .fold(0u32, |w, (i, &c)| w | u32::from(c) << (8 * i));
            if word & mask != value {
                problems.push(format!("the filter {mask:#x}/{value:#x} rejects {word:#x}"));
            }
        }
    }
}

/// Compiles and walks one row in both forms; the differences from node, as text.
fn check_row(row: &Row, budget_hits: &mut usize) -> Vec<String> {
    let mut out = Vec::new();
    let f = flags(&row.flags);
    let pattern = match parse(&row.pattern, f) {
        Ok(p) => p,
        Err(e) => return vec![format!("parse: {e:?}")],
    };
    let latin1 = row.subject.iter().all(|&c| c <= 0xff);
    for one_byte in [false, true] {
        if one_byte && !latin1 {
            continue;
        }
        let form = if one_byte { "one-byte" } else { "two-byte" };
        let options = CompileOptions::for_source(one_byte, row.pattern.len());
        let program = match compile(&pattern, f, options) {
            Ok(p) => p,
            Err(e) => {
                out.push(format!("{form}: compile: {e:?}"));
                continue;
            }
        };
        if let Err(e) = program.validate() {
            out.push(format!("{form}: validate: {e}"));
        }
        let mut walker = Walker::new(&program, &row.subject, STEPS);
        let Ok(got) = walker.exec() else {
            *budget_hits += 1;
            continue;
        };
        let mut problems = std::mem::take(&mut walker.problems);
        if got != row.expected {
            out.push(format!("{form}: got {got:?}, node {:?}", row.expected));
        }
        check_match(&program, &row.subject, &got, &mut problems);
        out.extend(problems.into_iter().map(|p| format!("{form}: {p}")));
    }
    out
}

fn check_rows(text: String) -> (usize, usize, Vec<String>) {
    std::thread::Builder::new()
        .stack_size(WALK_STACK)
        .spawn(move || {
            let rows = rows(&text);
            let mut budget_hits = 0;
            let mut report = Vec::new();
            for row in &rows {
                for d in check_row(row, &mut budget_hits) {
                    report.push(format!(
                        "/{}/{} on \"{}\": {d}",
                        String::from_utf16_lossy(&row.pattern),
                        row.flags,
                        String::from_utf16_lossy(&row.subject).escape_debug()
                    ));
                }
            }
            (rows.len(), budget_hits, report)
        })
        .expect("spawn")
        .join()
        .expect("the walk thread")
}

#[test]
fn compiled_programs_match_node() {
    let (n, budget_hits, report) =
        check_rows(include_str!("../tests/compiler_exec.txt").to_owned());
    assert!(n >= 150, "{n} fixture rows");
    assert_eq!(budget_hits, 0, "no fixture row runs out of steps");
    assert!(
        report.is_empty(),
        "{} differences:\n{}",
        report.len(),
        report.join("\n")
    );
}

/// The fixture form on a file named by `EP9_RX1_EXEC` (made with node from the
/// parse-equality pairs, `EP9_RX1_PAIRS` of tests/parse_equality.rs). A row node refused
/// (`error` as the result) or one out of steps is counted, not compared.
#[test]
#[ignore = "needs EP9_RX1_EXEC=<file> (node results of the test262 pairs); run with --ignored"]
fn compiled_programs_match_node_on_a_corpus_file() {
    let path = std::env::var("EP9_RX1_EXEC").expect("EP9_RX1_EXEC=<file>");
    let text = std::fs::read_to_string(path).expect("the exec file");
    let kept: String = text
        .lines()
        .filter(|l| !l.ends_with("\terror"))
        .flat_map(|l| [l, "\n"])
        .collect();
    let (n, budget_hits, report) = check_rows(kept);
    println!(
        "rows {n}, out of steps {budget_hits}, differences {}",
        report.len()
    );
    assert!(
        report.is_empty(),
        "{} differences:\n{}",
        report.len(),
        report.join("\n")
    );
}

fn program(pattern: &str, f: &str, one_byte: bool) -> Program {
    let pattern_units: Vec<u16> = pattern.encode_utf16().collect();
    let parsed = parse(&pattern_units, flags(f)).expect("parses");
    compile(
        &parsed,
        flags(f),
        CompileOptions::for_source(one_byte, pattern_units.len()),
    )
    .expect("compiles")
}

#[test]
fn eats_at_least_values() {
    // (pattern, flags, V8 EatsAtLeast of the start node, possibly at the start)
    let cases = [
        ("abc", "y", 3),
        ("a|bc", "y", 1),
        ("a{2,3}", "y", 2),
        ("(?=abc)a", "y", 1),
        ("a*", "y", 0),
        ("(?<=a)b", "y", 1),
        ("\\bab", "y", 2),
        ("abc", "", 3),
        ("x*abc", "", 3),
    ];
    for (p, f, want) in cases {
        for one_byte in [false, true] {
            let prog = program(p, f, one_byte);
            assert_eq!(
                prog.eats_at_least(prog.start(), false),
                want,
                "/{p}/{f} {one_byte}"
            );
        }
    }
}

#[test]
fn search_loop_and_anchor() {
    assert!(program("abc", "", false).has_search_prefix());
    assert!(!program("^abc", "", false).has_search_prefix());
    assert!(!program("abc", "y", false).has_search_prefix());
    assert!(program("^abc", "m", false).has_search_prefix());
    assert!(!program("^a|^b", "", false).has_search_prefix());
    assert!(program("^a|b", "", false).has_search_prefix());
}

#[test]
fn one_byte_quick_check_filters() {
    let f = program("^abc", "", true).quick_check_filters();
    assert_eq!(f.mask_value, Some((0x00ff_ffff, 0x0063_6261)));
    let f = program("^abc", "i", true).quick_check_filters();
    assert_eq!(f.mask_value, Some((0x00df_dfdf, 0x0043_4241)));
    let f = program("^[ab]", "", true).quick_check_filters();
    let reject = f.reject_bitset.expect("a reject set");
    for c in 0..256u32 {
        let rejected = reject[(c / 32) as usize] >> (c % 32) & 1 != 0;
        assert_eq!(rejected, c != 0x61 && c != 0x62, "{c:#x}");
    }
    // A search loop, a two-byte program or `m`: no filters.
    assert_eq!(
        program("abc", "", true).quick_check_filters(),
        Default::default()
    );
    assert_eq!(
        program("^abc", "", false).quick_check_filters(),
        Default::default()
    );
    assert_eq!(
        program("^abc", "m", true).quick_check_filters(),
        Default::default()
    );
}

#[test]
fn a_one_byte_program_of_a_non_latin1_atom_cannot_match() {
    let prog = program("\\u0100", "", true);
    let subject: Vec<u16> = "\u{ff}".encode_utf16().collect();
    assert_eq!(Walker::new(&prog, &subject, STEPS).exec(), Ok(None));
    // In the two-byte program the atom is there.
    let prog = program("\\u0100", "", false);
    assert!(prog.nodes().iter().any(|n| matches!(
        &n.kind,
        compiler::node::NodeKind::Text(t) if t.elements.iter().any(|e| matches!(
            &e.kind,
            compiler::node::TextElementKind::Atom(a) if a[..] == [0x100]
        ))
    )));
}

#[test]
fn too_many_registers_is_too_large() {
    // 32767 captures use registers 0..65535; a look-ahead needs two more (V8
    // `RegExpCompiler::AllocateRegister`, `reg_exp_too_big_`).
    let mut p = "(a)".repeat(MAX_CAPTURES as usize);
    p.push_str("(?=a)");
    let units: Vec<u16> = p.encode_utf16().collect();
    let parsed = parse(&units, Flags::empty()).expect("parses");
    let options = CompileOptions::for_source(false, units.len());
    assert_eq!(
        compile(&parsed, Flags::empty(), options),
        Err(Error::TooLarge)
    );
    let parsed = parse_str("(a)(?=a)", Flags::empty()).expect("parses");
    assert!(compile(&parsed, Flags::empty(), options).is_ok());
}

#[test]
fn a_long_chain_compiles_on_the_test_thread_stack() {
    // The analysis keeps its own stack (V8 recurses and reports a stack overflow).
    let p = "a?".repeat(20_000);
    let units: Vec<u16> = p.encode_utf16().collect();
    let parsed = parse(&units, Flags::empty()).expect("parses");
    let prog = compile(
        &parsed,
        Flags::empty(),
        CompileOptions::for_source(false, units.len()),
    )
    .expect("compiles");
    prog.validate().expect("valid");
    assert!(!CompileOptions::for_source(false, units.len()).optimize);
}

#[test]
fn every_reachable_node_is_analyzed() {
    for (p, f) in [
        ("(a|b)*c(?=d)(?<!e)\\1", ""),
        ("[\\p{L}--[a-z]]+\\b", "vi"),
        ("(?:\\u{1F600}|.)+?$", "um"),
    ] {
        for one_byte in [false, true] {
            let prog = program(p, f, one_byte);
            prog.validate().expect("valid");
            for id in prog.reachable() {
                assert!(
                    prog.node(id).info().been_analyzed,
                    "/{p}/{f} node {}",
                    id.index()
                );
            }
        }
    }
}

#[test]
fn the_walker_reports_a_wrong_eats_at_least_value() {
    // The check of the analysis is live: a start node that claims 4 characters fails on a
    // 3-character match.
    let mut prog = program("abc", "y", true);
    let subject: Vec<u16> = "abc".encode_utf16().collect();
    let mut walker = Walker::new(&prog, &subject, STEPS);
    assert!(walker.exec().expect("in budget").is_some());
    assert!(walker.problems.is_empty(), "{:?}", walker.problems);
    let start = prog.start().index();
    prog.nodes[start].eats = compiler::node::EatsAtLeastInfo::new(4);
    let mut walker = Walker::new(&prog, &subject, STEPS);
    assert!(walker.exec().expect("in budget").is_some());
    assert_eq!(walker.problems.len(), 1, "{:?}", walker.problems);
}
