//! Stack probe: measures stack bytes per depth unit for every nesting form and every phase.
//! The constants in `stack.rs` come from its output. Run it on demand:
//!
//! ```text
//! cargo test -p ep9_frontend           probe::measure -- --ignored --nocapture   (debug)
//! cargo test -p ep9_frontend --release probe::measure -- --ignored --nocapture   (release)
//! ```
//!
//! Method: stack painting. A thread with a 2 GiB stack fills the region below its stack pointer
//! with a pattern, runs one phase, and finds the lowest word that changed. Two nesting counts
//! give the slope (bytes per nesting level), which removes the fixed base cost.

use std::fmt::Write as _;
use std::hint::black_box;

use oxc_allocator::Allocator;

use crate::forms::{FORMS, Form};
use crate::{Goal, SourceOpts, Strictness, depth, prescan};

const PATTERN: u64 = 0x5A5A_C3C3_A5A5_3C3C;
/// Bytes between the measuring frame and the painted region (the probe's own frame, red zone).
const GAP: usize = 16 << 10;

/// Paint `bytes` of stack below the caller, run `f`, return the deepest stack use of `f` in bytes.
///
/// Panics if `f` used the whole painted region (the measurement would be too low).
#[inline(never)]
fn measure<R>(bytes: usize, f: impl FnOnce() -> R) -> (usize, R) {
    let (used, result, full) = measure_or_full(bytes, f);
    assert!(!full, "the painted region ({bytes} B) is too small");
    (used, result)
}

/// As [`measure`], but reports a full region instead of panicking.
#[inline(never)]
fn measure_or_full<R>(bytes: usize, f: impl FnOnce() -> R) -> (usize, R, bool) {
    let anchor = 0u8;
    let sp = black_box(&raw const anchor) as usize;
    let low = (sp - bytes) & !7;
    let high = (sp - GAP) & !7;
    let mut addr = low;
    while addr < high {
        // SAFETY: `[low, high)` lies inside this thread's stack (the probe thread has a 2 GiB
        // stack and `bytes` is at most 1.5 GiB), below the current stack pointer minus a 64 KiB
        // gap, so no live frame and no red zone uses it. Volatile writes keep the compiler from
        // eliding or reordering them.
        unsafe { std::ptr::write_volatile(addr as *mut u64, PATTERN) };
        addr += 8;
    }
    let result = black_box(f());
    let mut addr = low;
    // SAFETY: as above: reads of this thread's own stack below every live frame.
    while addr < high && unsafe { std::ptr::read_volatile(addr as *const u64) } == PATTERN {
        addr += 8;
    }
    (sp.saturating_sub(addr), result, addr == low)
}

#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    estimate: u32,
    ast_depth: u32,
    prescan: usize,
    parse: usize,
    depth_pass: usize,
    semantic: usize,
    scope: usize,
}

fn sample(form: &Form, n: usize, paint: usize) -> Sample {
    let src = (form.build)(n);
    let opts = if form.module {
        SourceOpts::module()
    } else {
        SourceOpts::script(Strictness::Sloppy)
    };
    let module = opts.goal() == Goal::Module;
    let (prescan_bytes, est) = measure(paint, || prescan::estimate(src.as_bytes(), module));
    let allocator = Allocator::default();
    let source_type = if module {
        oxc_span::SourceType::mjs()
    } else {
        oxc_span::SourceType::script()
    };
    let options = oxc_parser::ParseOptions {
        parse_regular_expression: true,
        ..oxc_parser::ParseOptions::default()
    };
    let (parse_bytes, parsed) = measure(paint, || {
        oxc_parser::Parser::new(&allocator, &src, source_type)
            .with_options(options)
            .parse()
    });
    assert!(
        !parsed.diagnostics.has_errors(),
        "form {} n {n}: {:?}",
        form.name,
        parsed.diagnostics.errors().next()
    );
    let program = parsed.program;
    let (depth_bytes, ast) = measure(paint, || depth::check_ast_depth(&program, u32::MAX));
    let ast_depth = ast.expect("no cap");
    let (semantic_bytes, built) = measure(paint, || {
        oxc_semantic::SemanticBuilder::new_compiler().build(&program)
    });
    assert!(
        !built.diagnostics.has_errors(),
        "form {} n {n}: {:?}",
        form.name,
        built.diagnostics.errors().next()
    );
    let semantic = built.semantic;
    let fe = crate::FrontEnd {
        source: &src,
        opts,
        program: &program,
        semantic: &semantic,
    };
    let (scope_bytes, scopes) = measure(paint, || crate::scope::analyze(&fe));
    assert!(scopes.is_ok(), "form {} n {n}: {scopes:?}", form.name);
    Sample {
        estimate: est.depth,
        ast_depth,
        prescan: prescan_bytes,
        parse: parse_bytes,
        depth_pass: depth_bytes,
        semantic: semantic_bytes,
        scope: scope_bytes,
    }
}

fn per(delta_bytes: usize, delta_units: u32) -> String {
    if delta_units == 0 {
        if delta_bytes > 4096 {
            format!("{delta_bytes}B/0!")
        } else {
            "-".into()
        }
    } else {
        format!("{:.0}", delta_bytes as f64 / f64::from(delta_units))
    }
}

/// Stack growth from n1 to n2 that is noise, not a slope (one page).
const FLAT: usize = 4 << 10;

/// Parse stack one pre-scan unit may use: its share of the worker stack, keeping the 1.5x
/// margin of `stack.rs` (4,663 B debug, 2,615 B release).
const PARSE_UNIT_BUDGET: usize =
    crate::stack::WORKER_STACK_BYTES / 3 * 2 / crate::PRESCAN_CAP as usize;

/// Pass stack one AST level may use: its share of the worker stack, keeping the 1.5x margin of
/// `stack.rs`, as `bytecodegen::stack_tests` does for codegen (5,829 B debug, 3,269 B release).
const PASS_LEVEL_BUDGET: usize = crate::stack::WORKER_STACK_BYTES / 3 * 2 / crate::AST_CAP as usize;

/// `None` if `delta_bytes` over `delta_units` fits `budget` per unit; else what is wrong. No
/// units means the stack must stay flat (the cap counts nothing for this form).
fn over_budget(delta_bytes: usize, delta_units: u32, budget: usize) -> Option<String> {
    if delta_units == 0 {
        (delta_bytes > FLAT).then(|| format!("{delta_bytes} B with no units"))
    } else {
        let per = delta_bytes / delta_units as usize;
        (per > budget).then(|| format!("{per} B/unit > budget {budget}"))
    }
}

/// Measures every form, and fails when a slope does not fit its share of the worker stack:
/// parse per pre-scan unit, the depth pass, oxc_semantic and the scope pass per AST level.
#[test]
#[ignore = "measurement; run on demand with --ignored --nocapture"]
fn measure_forms() {
    let (n1, n2) = (500usize, 1500usize);
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let (out, over) = std::thread::Builder::new()
        .stack_size(2 << 30)
        .spawn(move || {
            let mut over = Vec::new();
            let mut out = String::new();
            writeln!(
                out,
                "profile {profile}, target {}, n = {n1} -> {n2}",
                std::env::consts::ARCH
            )
            .unwrap();
            writeln!(
                out,
                "| form | dE/dn | dAST/dn | parse B/E | parse B/n | depth B/AST | semantic B/AST | scope B/AST | E@n2 | AST@n2 | parse base | semantic base | scope base | prescan B@n2 |"
            )
            .unwrap();
            writeln!(out, "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|").unwrap();
            for form in FORMS {
                let paint = 384 << 20;
                let a = sample(form, n1, paint);
                let b = sample(form, n2, paint);
                let de = b.estimate - a.estimate;
                let dast = b.ast_depth - a.ast_depth;
                let dn = u32::try_from(n2 - n1).unwrap();
                // Base = the stack at n2 minus the slope times the units at n2.
                let base = |at_b: usize, at_a: usize, units_b: u32, units_a: u32| -> i64 {
                    let du = i64::from(units_b) - i64::from(units_a);
                    if du == 0 {
                        return i64::try_from(at_b).unwrap();
                    }
                    let slope = (i64::try_from(at_b).unwrap() - i64::try_from(at_a).unwrap()) / du;
                    i64::try_from(at_b).unwrap() - slope * i64::from(units_b)
                };
                writeln!(
                    out,
                    "| {} | {:.2} | {:.2} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                    form.name,
                    f64::from(de) / f64::from(dn),
                    f64::from(dast) / f64::from(dn),
                    per(b.parse.saturating_sub(a.parse), de),
                    per(b.parse.saturating_sub(a.parse), dn),
                    per(b.depth_pass.saturating_sub(a.depth_pass), dast),
                    per(b.semantic.saturating_sub(a.semantic), dast),
                    per(b.scope.saturating_sub(a.scope), dast),
                    b.estimate,
                    b.ast_depth,
                    base(b.parse, a.parse, b.estimate, a.estimate),
                    base(b.semantic, a.semantic, b.ast_depth, a.ast_depth),
                    base(b.scope, a.scope, b.ast_depth, a.ast_depth),
                    // The pre-scan is a loop: its stack does not grow with the input.
                    b.prescan,
                )
                .unwrap();
                let pass = |at_b: usize, at_a: usize| {
                    over_budget(at_b.saturating_sub(at_a), dast, PASS_LEVEL_BUDGET)
                };
                for (phase, problem) in [
                    (
                        "parse",
                        over_budget(b.parse.saturating_sub(a.parse), de, PARSE_UNIT_BUDGET),
                    ),
                    ("depth pass", pass(b.depth_pass, a.depth_pass)),
                    ("semantic", pass(b.semantic, a.semantic)),
                    ("scope pass", pass(b.scope, a.scope)),
                    ("pre-scan", over_budget(b.prescan.saturating_sub(a.prescan), 0, 0)),
                ] {
                    if let Some(problem) = problem {
                        over.push(format!("{} {phase}: {problem}", form.name));
                    }
                }
            }
            (out, over)
        })
        .unwrap()
        .join()
        .unwrap();
    println!("{out}");
    assert!(
        over.is_empty(),
        "stack slopes above the worker stack share:\n{}",
        over.join("\n")
    );
}

/// Literal starts (regex, string, template) in oxc's AST.
#[derive(Default)]
struct OxcLiterals(Vec<(prescan::LitKind, u32)>);

impl<'a> oxc_ast_visit::Visit<'a> for OxcLiterals {
    fn visit_reg_exp_literal(&mut self, it: &oxc_ast::ast::RegExpLiteral<'a>) {
        self.0.push((prescan::LitKind::Regex, it.span.start));
    }
    fn visit_string_literal(&mut self, it: &oxc_ast::ast::StringLiteral<'a>) {
        self.0.push((prescan::LitKind::String, it.span.start));
    }
    fn visit_template_literal(&mut self, it: &oxc_ast::ast::TemplateLiteral<'a>) {
        self.0.push((prescan::LitKind::Template, it.span.start));
        oxc_ast_visit::walk::walk_template_literal(self, it);
    }
}

/// A corpus check over no files proves nothing: fail when a checkout under `third_party/` is
/// missing or empty, and name the script that syncs it.
pub(crate) fn assert_checkout(dir: &str, files: usize) {
    let script = if dir.starts_with("test262") {
        "scripts/sync-test262.sh"
    } else {
        "scripts/sync-bench.sh"
    };
    assert!(
        files > 0,
        "third_party/{dir} has no .js or .mjs file: run `bash {script}` first"
    );
}

fn corpus_files() -> Vec<(std::path::PathBuf, bool)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party");
    let mut out = Vec::new();
    for top in [
        "test262/test",
        "test262/harness",
        "bench-octane",
        "bench-v8v7",
        "bench-jetstream",
    ] {
        let before = out.len();
        let mut stack = vec![root.join(top)];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    if path
                        .file_name()
                        .is_some_and(|n| n == ".git" || n == "node_modules")
                    {
                        continue;
                    }
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "js" || e == "mjs") {
                    let module = path.extension().is_some_and(|e| e == "mjs");
                    out.push((path, module));
                }
            }
        }
        assert_checkout(top, out.len() - before);
    }
    out
}

/// Parse stack that does not grow with the estimate: the 16 KiB `GAP` that every measurement
/// includes, oxc_parser's base and the regex parser's base. Corpus worst against
/// `PARSE_UNIT_BUDGET` (aarch64, 2026-10-21): 13,776 B release (an empty file), 35,568 B debug
/// (a `\p{...}` regex).
const PARSE_BASE: usize = 64 << 10;

/// Every valid file of test262 and the benchmark pins: the scanner finds exactly oxc's regex,
/// string and template literals, and the real parse stack stays within the model
/// `PARSE_BASE + estimate x PARSE_UNIT_BUDGET`. Run on demand (needs the third_party
/// checkouts; fails without them):
/// `cargo test -p ep9_frontend --release probe::scanner -- --ignored --nocapture`
#[test]
#[ignore = "corpus check; needs third_party checkouts; run on demand"]
fn scanner_matches_oxc_on_corpus() {
    let files = corpus_files();
    let (report, checked, mismatches, over_model) = std::thread::Builder::new()
        .stack_size(2 << 30)
        .spawn(move || {
            let mut checked = 0usize;
            let mut literals = 0usize;
            let mut mismatches: Vec<String> = Vec::new();
            let mut over_model: Vec<String> = Vec::new();
            let mut worst_residual: (i64, String, u32, usize) = (i64::MIN, String::new(), 0, 0);
            let mut worst_per_unit: (f64, String, u32, usize) = (0.0, String::new(), 0, 0);
            let unit = i64::try_from(PARSE_UNIT_BUDGET).unwrap();
            for (path, ext_module) in &files {
                let Ok(src) = std::fs::read_to_string(path) else {
                    continue;
                };
                let module = *ext_module
                    || src.contains("flags: [module]")
                    || (src.contains("flags:")
                        && src.contains("module")
                        && src.contains("---*/")
                        && src
                            .split("---*/")
                            .next()
                            .is_some_and(|fm| fm.contains("module")));
                let negative = src.contains("negative:");
                if negative {
                    continue;
                }
                let source_type = if module {
                    oxc_span::SourceType::mjs()
                } else {
                    oxc_span::SourceType::script()
                };
                let est = prescan::estimate(src.as_bytes(), module);
                let paint = (512usize << 10).max((usize::try_from(est.depth).unwrap() * 8) << 10);
                let allocator = Allocator::default();
                let options = oxc_parser::ParseOptions {
                    parse_regular_expression: true,
                    ..oxc_parser::ParseOptions::default()
                };
                let (used, parsed, full) = measure_or_full(paint, || {
                    oxc_parser::Parser::new(&allocator, &src, source_type)
                        .with_options(options)
                        .parse()
                });
                assert!(!full, "{}: painted region too small", path.display());
                if parsed.diagnostics.has_errors() {
                    continue;
                }
                checked += 1;
                let name = path.display().to_string();
                let residual = i64::try_from(used).unwrap() - unit * i64::from(est.depth);
                if residual > worst_residual.0 {
                    worst_residual = (residual, name.clone(), est.depth, used);
                }
                if residual > i64::try_from(PARSE_BASE).unwrap() {
                    over_model.push(format!(
                        "{name}: estimate {}, used {used} B, residual {residual} B",
                        est.depth
                    ));
                }
                if est.depth > 20 {
                    let per_unit = used as f64 / f64::from(est.depth);
                    if per_unit > worst_per_unit.0 {
                        worst_per_unit = (per_unit, name.clone(), est.depth, used);
                    }
                }
                let mut oxc = OxcLiterals::default();
                oxc_ast_visit::Visit::visit_program(&mut oxc, &parsed.program);
                let mut theirs = oxc.0;
                theirs.sort_unstable();
                theirs.dedup();
                let mut ours = prescan::literal_tokens(src.as_bytes(), module);
                ours.sort_unstable();
                literals += theirs.len();
                if ours != theirs {
                    let missing: Vec<_> = theirs
                        .iter()
                        .filter(|t| !ours.contains(t))
                        .take(3)
                        .collect();
                    let extra: Vec<_> = ours
                        .iter()
                        .filter(|t| !theirs.contains(t))
                        .take(3)
                        .collect();
                    let show = |at: u32| {
                        let at = at as usize;
                        let from = src.floor_char_boundary(at.saturating_sub(30));
                        let to = src.ceil_char_boundary((at + 30).min(src.len()));
                        src[from..to].replace('\n', "\\n")
                    };
                    let detail = missing
                        .iter()
                        .map(|(k, at)| format!("missing {k:?}@{at} `{}`", show(*at)))
                        .chain(
                            extra
                                .iter()
                                .map(|(k, at)| format!("extra {k:?}@{at} `{}`", show(*at))),
                        )
                        .collect::<Vec<_>>()
                        .join("; ");
                    mismatches.push(format!("{name}: {detail}"));
                }
            }
            let mut out = String::new();
            writeln!(
                out,
                "files {} (valid, checked {checked}), oxc literals {literals}",
                files.len()
            )
            .unwrap();
            writeln!(out, "token mismatches: {}", mismatches.len()).unwrap();
            for m in mismatches.iter().take(40) {
                writeln!(out, "  {m}").unwrap();
            }
            writeln!(
                out,
                "worst residual (used - {unit} x estimate): {} B in {} (estimate {}, used {})",
                worst_residual.0, worst_residual.1, worst_residual.2, worst_residual.3
            )
            .unwrap();
            writeln!(
                out,
                "worst bytes per unit (estimate > 20): {:.0} in {} (estimate {}, used {})",
                worst_per_unit.0, worst_per_unit.1, worst_per_unit.2, worst_per_unit.3
            )
            .unwrap();
            (out, checked, mismatches, over_model)
        })
        .unwrap()
        .join()
        .unwrap();
    println!("{report}");
    // Coverage: files oxc rejects (negative tests, module code run as a script) are skipped.
    assert_eq!(
        checked, 49_522,
        "files checked changed: review the skipped files, then update this count"
    );
    assert!(
        mismatches.is_empty(),
        "{} files where the scanner's literal tokens differ from oxc:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    assert!(
        over_model.is_empty(),
        "{} files whose parse stack is above {PARSE_BASE} + estimate x {PARSE_UNIT_BUDGET} B:\n{}",
        over_model.len(),
        over_model.join("\n")
    );
}

/// test262 front matter: (module goal, strict only).
fn test262_flags(src: &str, ext_module: bool) -> (bool, bool) {
    let front = src
        .split("/*---")
        .nth(1)
        .and_then(|rest| rest.split("---*/").next())
        .unwrap_or("");
    let flags = front
        .lines()
        .find(|l| l.trim_start().starts_with("flags:"))
        .unwrap_or("");
    (
        ext_module || flags.contains("module"),
        flags.contains("onlyStrict"),
    )
}

/// Every valid file of test262 and the benchmark pins through the scope pass: no panic, no
/// abort, every `IdentifierReference` visited, the same declaration as oxc's resolution (or
/// one of the reviewed differences), and the same result twice (determinism). Run on demand
/// (needs the third_party checkouts; fails without them):
/// `cargo test -p ep9_frontend --release probe::scope_pass -- --ignored --nocapture`
#[test]
#[ignore = "corpus check; needs third_party checkouts; run on demand"]
fn scope_pass_matches_oxc_on_corpus() {
    use std::time::{Duration, Instant};

    let files = corpus_files();
    let (mut ok, mut rejected, mut references, mut unvisited, mut not_equal) = (0, 0, 0, 0, 0);
    let mut time = Duration::ZERO;
    let mut mismatches: Vec<String> = Vec::new();
    let mut by_why = std::collections::BTreeMap::new();
    let mut rejected_by = std::collections::BTreeMap::new();
    for (path, ext_module) in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        if src.contains("negative:") {
            continue;
        }
        let (module, only_strict) = test262_flags(&src, *ext_module);
        let opts = if module {
            SourceOpts::module()
        } else if only_strict {
            SourceOpts::script(Strictness::Strict)
        } else {
            SourceOpts::script(Strictness::Sloppy)
        };
        let owned = src.clone();
        let result = crate::with_front_end(&src, opts, move |fe| {
            let start = Instant::now();
            let first = crate::scope::analyze(fe)?;
            let elapsed = start.elapsed();
            let second = crate::scope::analyze(fe)?;
            let (found, missed) = crate::scope::tests::oxc_mismatches(fe, &first);
            let detail: Vec<(crate::scope::tests::Why, String)> = found
                .iter()
                .map(|m| {
                    let at = m.at as usize;
                    let from = owned.floor_char_boundary(at.saturating_sub(40));
                    let to = owned.ceil_char_boundary((at + 30).min(owned.len()));
                    (
                        m.why,
                        format!(
                            "{:?} {} @{} ep9 {:?} oxc {:?} `{}`",
                            m.why,
                            m.name,
                            m.at,
                            m.ep9,
                            m.oxc,
                            owned[from..to].replace('\n', "\\n")
                        ),
                    )
                })
                .collect();
            Ok((
                elapsed,
                first.references().len(),
                missed,
                first == second,
                detail,
            ))
        });
        match result {
            Ok((elapsed, refs, missed, equal, detail)) => {
                ok += 1;
                time += elapsed;
                references += refs;
                unvisited += missed;
                not_equal += usize::from(!equal);
                for (why, d) in detail {
                    *by_why.entry(why).or_insert(0usize) += 1;
                    mismatches.push(format!("{}: {d}", path.display()));
                }
            }
            Err(e) => {
                rejected += 1;
                let key = match &e {
                    crate::CompileError::Syntax(s) => format!("syntax: {}", s.first().message),
                    crate::CompileError::Range(r) => format!("range: {:?}", r.kind),
                };
                let short = path.display().to_string();
                let short = short
                    .split("third_party/")
                    .nth(1)
                    .unwrap_or(&short)
                    .to_owned();
                rejected_by.entry(key).or_insert_with(Vec::new).push(short);
            }
        }
    }
    println!("files analyzed {ok}, rejected by the front end {rejected}");
    for (key, files) in &rejected_by {
        println!("  rejected {} x {key}, e.g. {}", files.len(), files[0]);
    }
    println!(
        "references {references}; oxc references or scopes with no ep9 counterpart {unvisited}; different on a second run {not_equal}"
    );
    println!("scope pass time {:.2?} total", time);
    println!(
        "references resolved unlike oxc: {} {by_why:?}",
        mismatches.len()
    );
    // One example per file, with the count.
    let mut per_file: Vec<(String, usize, String)> = Vec::new();
    for m in &mismatches {
        let (file, detail) = m.split_once(": ").unwrap();
        match per_file.last_mut() {
            Some(last) if last.0 == file => last.1 += 1,
            _ => per_file.push((file.to_owned(), 1, detail.to_owned())),
        }
    }
    println!("files with differences: {}", per_file.len());
    for (file, count, example) in per_file
        .iter()
        .filter(|(_, _, e)| e.starts_with("Other"))
        .chain(per_file.iter().take(3))
    {
        let short = file.split("third_party/").nth(1).unwrap_or(file);
        println!("  {short} x{count}: {example}");
    }
    // Coverage: a rejected file drops out of every check below. At the pinned corpus the
    // rejections are module code run as a script (`_FIXTURE.js` files and bench modules carry
    // no module flag) and the annexB call-expression assignment targets oxc rejects.
    assert_eq!(
        (ok, rejected),
        (49_517, 267),
        "files analyzed / rejected changed: review the rejections above, then update these \
         counts and H01"
    );
    assert_eq!(
        unvisited, 0,
        "oxc references or scopes with no ep9 counterpart (BytecodeGen looks them up)"
    );
    assert_eq!(
        not_equal, 0,
        "files with a different ScopeAnalysis on the second run"
    );
    // Only the reviewed classes, at their reviewed counts (H01 "F1 corpus"). `Why::Other` is a
    // bug in ep9 or in oxc.
    let reviewed = std::collections::BTreeMap::from([
        (crate::scope::tests::Why::SplitBody, 54),
        (crate::scope::tests::Why::AnnexB, 488),
        (crate::scope::tests::Why::CatchVar, 4),
    ]);
    assert_eq!(
        by_why, reviewed,
        "references resolved unlike oxc changed: review the examples above, then update this \
         table and H01"
    );
}

/// The argument text of every `eval(`, `Function(` and `evalScript(` call in `src`.
fn eval_arguments(src: &str) -> Vec<&str> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    for needle in ["eval(", "Function(", "evalScript("] {
        let mut from = 0;
        while let Some(i) = src[from..].find(needle) {
            let at = from + i;
            from = at + needle.len();
            // `eval(` must not be the tail of a longer identifier (`myeval(`).
            if needle == "eval("
                && at > 0
                && (bytes[at - 1].is_ascii_alphanumeric()
                    || bytes[at - 1] == b'_'
                    || bytes[at - 1] == b'$')
            {
                continue;
            }
            let open = at + needle.len() - 1;
            if let Some(close) = matching_paren(bytes, open) {
                out.push(&src[open + 1..close]);
            }
        }
    }
    out
}

/// Index of the `)` that closes the `(` at `open`, skipping strings, templates and comments.
fn matching_paren(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            q @ (b'"' | b'\'' | b'`') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Code units of `\uXXXX` / `\u{X}` escapes with an odd number of backslashes (real escapes in
/// the literal, not escaped backslashes), as (start, end, value).
fn unicode_escapes(text: &str) -> Vec<(usize, usize, u32)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && b[i] == b'\\' {
            i += 1;
        }
        if (i - start) % 2 == 0 || b.get(i) != Some(&b'u') {
            continue;
        }
        let esc_start = i - 1;
        i += 1;
        let (digits, end) = if b.get(i) == Some(&b'{') {
            let close = text[i..].find('}').map_or(i, |c| i + c);
            (&text[i + 1..close], close + 1)
        } else {
            (&text[i..(i + 4).min(text.len())], i + 4)
        };
        if let Ok(v) = u32::from_str_radix(digits, 16) {
            out.push((esc_start, end.min(text.len()), v));
        }
        i = end.min(text.len());
    }
    out
}

/// The literal text contains a surrogate escape that is not half of a lead+trail pair.
fn has_lone_surrogate_escape(text: &str) -> bool {
    let units = unicode_escapes(text);
    let lead = |v: u32| (0xD800..=0xDBFF).contains(&v);
    let trail = |v: u32| (0xDC00..=0xDFFF).contains(&v);
    units.iter().enumerate().any(|(k, &(start, end, v))| {
        if lead(v) {
            !units
                .get(k + 1)
                .is_some_and(|&(s, _, w)| s == end && trail(w))
        } else if trail(v) {
            !(k > 0 && units[k - 1].1 == start && lead(units[k - 1].2))
        } else {
            false
        }
    })
}

/// P4 baseline (01-front-end §7): test262 tests that pass source text with a lone surrogate to
/// `eval`, `Function` or `$262.evalScript`. Such source cannot be UTF-8; it needs the shim.
/// Fails without the test262 checkout, and when the set differs from the reviewed baseline.
/// `cargo test -p ep9_frontend --release probe::p4 -- --ignored --nocapture`
#[test]
#[ignore = "corpus count; needs third_party/test262; run on demand"]
fn p4_lone_surrogate_eval_count() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/test262/test");
    let mut files = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "js") {
                files.push(path);
            }
        }
    }
    assert_checkout("test262/test", files.len());
    files.sort();
    let (mut literal, mut generated, mut possible) = (Vec::new(), Vec::new(), Vec::new());
    // A `for` header that runs up to 0xFFFF, or code units built from 4 hex digits (S7.4_A5).
    let loops_all_code_units = |src: &str| {
        let bounded_loop = src.lines().any(|line| {
            line.trim_start().starts_with("for")
                && ["0xffff", "0xFFFF", "65535", "0x10000", "65536"]
                    .iter()
                    .any(|n| line.contains(n))
        });
        (src.contains("fromCharCode") || src.contains("fromCodePoint"))
            && (bounded_loop || src.contains("fromCharCode(\"0x\""))
    };
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        let args = eval_arguments(&src);
        if args.is_empty() {
            continue;
        }
        let name = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let computed = |a: &&str| {
            a.contains('+') || a.contains("${") || !a.trim_start().starts_with(['"', '\'', '`'])
        };
        if args.iter().any(|a| has_lone_surrogate_escape(a)) {
            literal.push(name);
        } else if args.iter().any(computed) && loops_all_code_units(&src) {
            generated.push(name);
        } else if has_lone_surrogate_escape(&src) {
            possible.push(name);
        }
    }
    println!("test262 files scanned: {}", files.len());
    println!(
        "A. lone surrogate in a literal eval/Function argument: {}",
        literal.len()
    );
    for f in &literal {
        println!("  {f}");
    }
    println!(
        "B. eval of String.fromCharCode over 0..0xFFFF: {}",
        generated.len()
    );
    for f in &generated {
        println!("  {f}");
    }
    println!(
        "P4 baseline count (A + B): {}",
        literal.len() + generated.len()
    );
    println!(
        "not counted, review by hand (eval call + lone surrogate elsewhere): {}",
        possible.len()
    );
    for f in &possible {
        println!("  {f}");
    }
    // The reviewed baseline (H05 "P4 baseline"): a change needs a new review by hand.
    let review =
        "the P4 set changed: review the files above by hand, then update this list and H05";
    assert_eq!(
        literal,
        [
            "language/literals/regexp/named-groups/invalid-lone-surrogate-groupname.js",
            "staging/sm/RegExp/unicode-class-raw.js",
            "staging/sm/RegExp/unicode-raw.js",
        ],
        "{review}"
    );
    assert_eq!(
        generated,
        [
            "annexB/built-ins/RegExp/RegExp-leading-escape-BMP.js",
            "annexB/built-ins/RegExp/RegExp-trailing-escape-BMP.js",
            "language/comments/S7.4_A5.js",
            "language/comments/S7.4_A6.js",
            "language/literals/regexp/S7.8.5_A1.1_T2.js",
            "language/literals/regexp/S7.8.5_A1.4_T2.js",
            "language/literals/regexp/S7.8.5_A2.1_T2.js",
            "language/literals/regexp/S7.8.5_A2.4_T2.js",
        ],
        "{review}"
    );
    assert_eq!(
        possible,
        [
            "staging/sm/RegExp/unicode-braced.js",
            "staging/sm/RegExp/unicode-class-braced.js",
            "staging/sm/RegExp/unicode-class-lead-trail.js",
            "staging/sm/RegExp/unicode-lead-trail.js",
            "staging/sm/String/unicode-braced.js",
        ],
        "{review}"
    );
}
