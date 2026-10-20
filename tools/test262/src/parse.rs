//! `ep9-test262 parse`: run every test through `ep9_frontend::check_early_errors`.
//!
//! ```text
//! test file --front matter--> runs by flags --> check_early_errors per run
//!   module      -> Module goal, source as-is
//!   raw         -> sloppy Script, source as-is
//!   onlyStrict  -> strict Script ("use strict";\n prefix, INTERPRETING.md)
//!   noStrict    -> sloppy Script
//!   (none)      -> sloppy Script and strict Script
//! negative.phase == parse : every run must report an early error
//! anything else           : every run must parse
//! ```
//!
//! Harness includes (`harness/*.js`) are not tests. Each one is parsed once per Script mode in
//! which some selected test loads it (assert.js and sta.js for every non-raw test,
//! doneprintHandle.js for async tests). They are reported apart from the test counts, but a
//! failure is a failure like any other. `test/harness/` and `test/staging/` are not run.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use ep9_frontend::{SourceOpts, Strictness, check_early_errors};

use crate::frontmatter::{self, Meta};
use crate::suite::{self, TestFile};
use crate::{expect, json, pool};

/// The test directories of a parse run (09-milestones §8 week 1).
pub const DIRS: [&str; 4] = ["language", "built-ins", "annexB", "intl402"];

/// INTERPRETING.md "Strict Mode": this exact prefix.
pub(crate) const STRICT_PREFIX: &str = "\"use strict\";\n";

/// Worker stack. oxc has no depth limit yet (01-front-end §6), so give it room.
pub const STACK_BYTES: usize = 256 << 20;

/// How strict runs become strict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrictBy {
    /// Prepend `"use strict";\n` to the source, as test262 says.
    Prefix,
    /// Pass `Strictness::Strict` to ep9_frontend and keep the source as-is.
    Opts,
}

pub struct Options {
    pub test262: PathBuf,
    pub expect: PathBuf,
    pub update: bool,
    pub json: Option<PathBuf>,
    pub threads: usize,
    pub strict_by: StrictBy,
    pub verbose: bool,
    /// Path prefixes relative to the test262 root. Empty = everything.
    pub filters: Vec<String>,
    pub triple: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Mode {
    Sloppy,
    Strict,
    Raw,
    Module,
}

impl Mode {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Mode::Sloppy => "sloppy",
            Mode::Strict => "strict",
            Mode::Raw => "raw",
            Mode::Module => "module",
        }
    }
}

struct Run {
    mode: Mode,
    ok: bool,
    /// The parser panicked. This is not a parse result (see `Outcome::infra_error`).
    panicked: bool,
    /// Why the run failed.
    detail: Option<String>,
}

struct Outcome {
    rel: String,
    module: bool,
    negative: bool,
    meta: Option<Meta>,
    runs: Vec<Run>,
    /// The file could not be read or its front matter could not be parsed.
    error: Option<String>,
}

impl Outcome {
    fn passed(&self) -> bool {
        self.error.is_none() && self.runs.iter().all(|r| r.ok)
    }

    /// Why the runner could not check this file: it could not be read, its front matter is bad,
    /// or the parser panicked. These are not parse results, so no expectation can list them.
    fn infra_error(&self) -> Option<String> {
        if let Some(e) = &self.error {
            return Some(e.clone());
        }
        let panics: Vec<String> = self
            .runs
            .iter()
            .filter(|r| r.panicked)
            .map(|r| format!("[{}] {}", r.mode.name(), r.detail.as_deref().unwrap_or("?")))
            .collect();
        (!panics.is_empty()).then(|| panics.join("; "))
    }

    fn failure_text(&self) -> String {
        if let Some(e) = &self.error {
            return e.clone();
        }
        let mut s = String::new();
        for run in self.runs.iter().filter(|r| !r.ok) {
            let _ = write!(
                s,
                "[{}] {}; ",
                run.mode.name(),
                run.detail.as_deref().unwrap_or("?")
            );
        }
        s.trim_end_matches("; ").to_string()
    }
}

#[derive(Clone, Copy, Default)]
struct Count {
    passed: usize,
    total: usize,
}

impl Count {
    fn add(&mut self, passed: bool) {
        self.total += 1;
        self.passed += usize::from(passed);
    }
}

pub(crate) fn modes(meta: &Meta) -> Vec<Mode> {
    if meta.has_flag("module") {
        vec![Mode::Module]
    } else if meta.has_flag("raw") {
        vec![Mode::Raw]
    } else if meta.has_flag("onlyStrict") {
        vec![Mode::Strict]
    } else if meta.has_flag("noStrict") {
        vec![Mode::Sloppy]
    } else {
        vec![Mode::Sloppy, Mode::Strict]
    }
}

/// `Ok(Some(error))` = early error, `Ok(None)` = parsed, `Err(message)` = panic.
fn has_early_error(src: &str, mode: Mode, strict_by: StrictBy) -> Result<Option<String>, String> {
    let result = catch_unwind(AssertUnwindSafe(|| match mode {
        Mode::Sloppy | Mode::Raw => check_early_errors(src, SourceOpts::script(Strictness::Sloppy)),
        Mode::Module => check_early_errors(src, SourceOpts::module()),
        Mode::Strict => match strict_by {
            StrictBy::Prefix => check_early_errors(
                &format!("{STRICT_PREFIX}{src}"),
                SourceOpts::script(Strictness::Sloppy),
            ),
            StrictBy::Opts => check_early_errors(src, SourceOpts::script(Strictness::Strict)),
        },
    }));
    match result {
        Ok(Ok(())) => Ok(None),
        Ok(Err(errors)) => Ok(Some(errors.to_string())),
        Err(panic) => Err(panic
            .downcast_ref::<&str>()
            .map(ToString::to_string)
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic".into())),
    }
}

fn run_modes(src: &str, modes: &[Mode], negative: bool, strict_by: StrictBy) -> Vec<Run> {
    modes
        .iter()
        .map(|&mode| {
            let result = has_early_error(src, mode, strict_by);
            let panicked = result.is_err();
            let (ok, detail) = match result {
                Ok(Some(_)) if negative => (true, None),
                Ok(None) if !negative => (true, None),
                Ok(Some(error)) => (false, Some(format!("unexpected early error: {error}"))),
                Ok(None) => (
                    false,
                    Some("parsed, but the test expects an early error".to_string()),
                ),
                Err(panic) => (false, Some(format!("panic: {panic}"))),
            };
            Run {
                mode,
                ok,
                panicked,
                detail,
            }
        })
        .collect()
}

fn run_test(file: &TestFile, strict_by: StrictBy) -> Outcome {
    let mut outcome = Outcome {
        rel: file.rel.clone(),
        module: false,
        negative: false,
        meta: None,
        runs: Vec::new(),
        error: None,
    };
    let src = match std::fs::read(&file.path).map(String::from_utf8) {
        Ok(Ok(src)) => src,
        Ok(Err(_)) => {
            outcome.error = Some("file is not valid UTF-8".into());
            return outcome;
        }
        Err(e) => {
            outcome.error = Some(format!("cannot read: {e}"));
            return outcome;
        }
    };
    let meta = match frontmatter::parse(&src) {
        Ok(meta) => meta,
        Err(e) => {
            outcome.error = Some(format!("front matter: {e}"));
            return outcome;
        }
    };
    outcome.module = meta.has_flag("module");
    outcome.negative = meta.is_parse_negative();
    outcome.runs = run_modes(&src, &modes(&meta), outcome.negative, strict_by);
    outcome.meta = Some(meta);
    outcome
}

/// Which harness include files the selected tests load, and in which Script modes.
fn include_needs(outcomes: &[Outcome]) -> BTreeMap<String, BTreeSet<Mode>> {
    let mut needs: BTreeMap<String, BTreeSet<Mode>> = BTreeMap::new();
    for outcome in outcomes {
        let Some(meta) = &outcome.meta else { continue };
        if meta.has_flag("raw") {
            continue; // raw tests load no harness file
        }
        // Includes run as global code: with the strict prefix in strict runs, sloppy beside a module.
        let script_modes: Vec<Mode> = if outcome.module {
            vec![Mode::Sloppy]
        } else {
            modes(meta)
        };
        let mut names = vec!["assert.js".to_string(), "sta.js".to_string()];
        if meta.has_flag("async") {
            names.push("doneprintHandle.js".to_string());
        }
        names.extend(meta.includes.iter().cloned());
        for name in names {
            needs
                .entry(name)
                .or_default()
                .extend(script_modes.iter().copied());
        }
    }
    needs
}

fn run_include(root: &Path, name: &str, modes: &BTreeSet<Mode>, strict_by: StrictBy) -> Outcome {
    let rel = format!("harness/{name}");
    let file = TestFile {
        path: root.join(&rel),
        rel,
    };
    let mut outcome = Outcome {
        rel: file.rel.clone(),
        module: false,
        negative: false,
        meta: None,
        runs: Vec::new(),
        error: None,
    };
    match std::fs::read(&file.path).map(String::from_utf8) {
        Ok(Ok(src)) => {
            let modes: Vec<Mode> = modes.iter().copied().collect();
            outcome.runs = run_modes(&src, &modes, false, strict_by);
        }
        Ok(Err(_)) => outcome.error = Some("file is not valid UTF-8".into()),
        Err(e) => outcome.error = Some(format!("cannot read: {e}")),
    }
    outcome
}

/// Run the `parse` command. Exit code: 0 = matches the expectations, 1 = differs, 2 = error.
pub fn run(opts: &Options) -> ExitCode {
    let started = Instant::now();
    let root = &opts.test262;
    let mut files = match suite::discover(root, &DIRS) {
        Ok(files) => files,
        Err(e) => {
            eprintln!("ep9-test262: cannot list tests in {}: {e}", root.display());
            return ExitCode::from(2);
        }
    };
    let full = opts.filters.is_empty();
    if !full {
        files.retain(|f| opts.filters.iter().any(|p| f.rel.starts_with(p.as_str())));
        if files.is_empty() {
            eprintln!("ep9-test262: no test matches {:?}", opts.filters);
            return ExitCode::from(2);
        }
    }
    if opts.update && !full {
        eprintln!("ep9-test262: --update-expectations needs a full run (no path filters)");
        return ExitCode::from(2);
    }

    let outcomes = pool::par_map(&files, opts.threads, STACK_BYTES, |f| {
        run_test(f, opts.strict_by)
    });
    let needs: Vec<(String, BTreeSet<Mode>)> = include_needs(&outcomes).into_iter().collect();
    let includes = pool::par_map(&needs, opts.threads, STACK_BYTES, |(name, modes)| {
        run_include(root, name, modes, opts.strict_by)
    });
    let wall = started.elapsed();
    let all = || outcomes.iter().chain(includes.iter());

    // Infrastructure errors: exit 2 before the expectations are compared or written.
    let infra: Vec<(&str, String)> = all()
        .filter_map(|o| o.infra_error().map(|e| (o.rel.as_str(), e)))
        .collect();
    if !infra.is_empty() {
        for (path, e) in &infra {
            eprintln!("ep9-test262: {path}: {e}");
        }
        eprintln!(
            "ep9-test262: {} files could not be checked (read error, bad front matter or parser panic); \
             the expectations were not compared or written",
            infra.len()
        );
        return ExitCode::from(2);
    }

    // Counts.
    let (mut neg, mut pos) = ([Count::default(); 2], [Count::default(); 2]);
    let mut runs = Count::default();
    let mut by_dir: BTreeMap<String, [Count; 2]> = BTreeMap::new();
    for o in &outcomes {
        for r in &o.runs {
            runs.add(r.ok);
        }
        let goal = usize::from(o.module);
        let bucket = if o.negative { &mut neg } else { &mut pos };
        bucket[goal].add(o.passed());
        by_dir
            .entry(suite::top_dir(&o.rel).to_string())
            .or_default()[usize::from(o.negative)]
        .add(o.passed());
    }
    let mut include_files = Count::default();
    let mut include_runs = Count::default();
    for o in &includes {
        include_files.add(o.passed());
        for r in &o.runs {
            include_runs.add(r.ok);
        }
    }

    // Expectations.
    let failing: BTreeSet<String> = all()
        .filter(|o| !o.passed())
        .map(|o| o.rel.clone())
        .collect();
    let checked: BTreeSet<String> = all().map(|o| o.rel.clone()).collect();
    let mut status = ExitCode::SUCCESS;
    let diff = if opts.update {
        if let Err(e) = expect::write(&opts.expect, &expect::parse_header(opts.triple), &failing) {
            eprintln!("ep9-test262: cannot write {}: {e}", opts.expect.display());
            return ExitCode::from(2);
        }
        expect::Diff::default()
    } else {
        match expect::read(&opts.expect) {
            Ok(expected) => expect::diff(&expected, &failing, &checked, full),
            Err(e) => {
                eprintln!(
                    "ep9-test262: cannot read expectations {}: {e}\n  (create it with --update-expectations)",
                    opts.expect.display()
                );
                return ExitCode::from(2);
            }
        }
    };
    if !diff.is_clean() {
        status = ExitCode::from(1);
    }

    // Report.
    let total = |c: [Count; 2]| Count {
        passed: c[0].passed + c[1].passed,
        total: c[0].total + c[1].total,
    };
    println!(
        "ep9-test262 parse: {} tests + {} harness includes, {} threads, target {}, strict runs by {}",
        outcomes.len(),
        includes.len(),
        opts.threads,
        opts.triple,
        match opts.strict_by {
            StrictBy::Prefix => "source prefix",
            StrictBy::Opts => "SourceOpts",
        }
    );
    println!("  test262: {}", root.display());
    println!("  {:<24}{:>8} / {:<8}", "", "passed", "total");
    let line = |label: &str, c: Count| println!("  {label:<24}{:>8} / {:<8}", c.passed, c.total);
    line("negatives, script goal", neg[0]);
    line("negatives, module goal", neg[1]);
    line("negatives, all", total(neg));
    line("positives, script goal", pos[0]);
    line("positives, module goal", pos[1]);
    line("positives, all", total(pos));
    line("runs (tests)", runs);
    line("harness include files", include_files);
    line("harness include runs", include_runs);
    println!("  by directory (positives, negatives):");
    for (dir, [p, n]) in &by_dir {
        println!(
            "    {dir:<12}{:>7} / {:<7}{:>6} / {:<6}",
            p.passed, p.total, n.passed, n.total
        );
    }
    let failures: Vec<&Outcome> = all().filter(|o| !o.passed()).collect();
    println!(
        "  failures: {} (new: {}, listed but passing: {}){}",
        failures.len(),
        diff.new_failures.len(),
        diff.now_passing.len(),
        if opts.update {
            format!("; wrote {}", opts.expect.display())
        } else {
            String::new()
        }
    );
    let details: BTreeMap<&str, String> = failures
        .iter()
        .map(|o| (o.rel.as_str(), o.failure_text()))
        .collect();
    for path in &diff.new_failures {
        println!(
            "  NEW FAILURE {path}: {}",
            details.get(path.as_str()).map_or("", String::as_str)
        );
    }
    for path in &diff.now_passing {
        println!(
            "  NOW PASSING (remove from {}): {path}",
            opts.expect.display()
        );
    }
    if opts.verbose {
        for (path, text) in &details {
            println!("  failure {path}: {text}");
        }
    }
    println!("  wall time: {:.2} s", wall.as_secs_f64());

    if let Some(path) = &opts.json {
        let summary = Summary {
            opts,
            neg,
            pos,
            runs,
            include_files,
            include_runs,
            by_dir: &by_dir,
            details: &details,
            diff: &diff,
            wall_s: wall.as_secs_f64(),
        };
        if let Err(e) = std::fs::write(path, summary.to_json()) {
            eprintln!("ep9-test262: cannot write {}: {e}", path.display());
            return ExitCode::from(2);
        }
    }
    status
}

struct Summary<'a> {
    opts: &'a Options,
    neg: [Count; 2],
    pos: [Count; 2],
    runs: Count,
    include_files: Count,
    include_runs: Count,
    by_dir: &'a BTreeMap<String, [Count; 2]>,
    details: &'a BTreeMap<&'a str, String>,
    diff: &'a expect::Diff,
    wall_s: f64,
}

impl Summary<'_> {
    fn to_json(&self) -> String {
        let count = |out: &mut String, c: Count| {
            let _ = write!(out, "{{\"passed\":{},\"total\":{}}}", c.passed, c.total);
        };
        let pair = |out: &mut String, c: [Count; 2]| {
            out.push_str("{\"script\":");
            count(out, c[0]);
            out.push_str(",\"module\":");
            count(out, c[1]);
            out.push_str(",\"all\":");
            count(
                out,
                Count {
                    passed: c[0].passed + c[1].passed,
                    total: c[0].total + c[1].total,
                },
            );
            out.push('}');
        };
        let mut out = String::from("{\"command\":\"parse\",\"test262\":");
        json::string(&mut out, &self.opts.test262.display().to_string());
        out.push_str(",\"target\":");
        json::string(&mut out, self.opts.triple);
        out.push_str(",\"strict_by\":");
        json::string(
            &mut out,
            if self.opts.strict_by == StrictBy::Prefix {
                "prefix"
            } else {
                "opts"
            },
        );
        let _ = write!(
            out,
            ",\"threads\":{},\"wall_seconds\":{:.3}",
            self.opts.threads, self.wall_s
        );
        out.push_str(",\"negatives\":");
        pair(&mut out, self.neg);
        out.push_str(",\"positives\":");
        pair(&mut out, self.pos);
        out.push_str(",\"runs\":");
        count(&mut out, self.runs);
        out.push_str(",\"harness_include_files\":");
        count(&mut out, self.include_files);
        out.push_str(",\"harness_include_runs\":");
        count(&mut out, self.include_runs);
        out.push_str(",\"by_dir\":{");
        for (i, (dir, [p, n])) in self.by_dir.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            json::string(&mut out, dir);
            out.push_str(":{\"positives\":");
            count(&mut out, *p);
            out.push_str(",\"negatives\":");
            count(&mut out, *n);
            out.push('}');
        }
        out.push_str("},\"failures\":[");
        for (i, (path, text)) in self.details.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("\n{\"path\":");
            json::string(&mut out, path);
            out.push_str(",\"detail\":");
            json::string(&mut out, text);
            out.push('}');
        }
        out.push_str("],\"expectations\":{\"file\":");
        json::string(&mut out, &self.opts.expect.display().to_string());
        out.push_str(",\"new_failures\":");
        json::string_array(&mut out, &self.diff.new_failures);
        out.push_str(",\"now_passing\":");
        json::string_array(&mut out, &self.diff.now_passing);
        out.push_str("}}\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(flags: &[&str]) -> Meta {
        Meta {
            flags: flags.iter().map(ToString::to_string).collect(),
            ..Meta::default()
        }
    }

    #[test]
    fn modes_follow_flags() {
        assert_eq!(modes(&meta(&[])), [Mode::Sloppy, Mode::Strict]);
        assert_eq!(modes(&meta(&["onlyStrict"])), [Mode::Strict]);
        assert_eq!(modes(&meta(&["noStrict"])), [Mode::Sloppy]);
        assert_eq!(modes(&meta(&["raw"])), [Mode::Raw]);
        assert_eq!(modes(&meta(&["module"])), [Mode::Module]);
        // test262 has 2 tests with both: module wins (the source is as-is in both).
        assert_eq!(modes(&meta(&["module", "raw"])), [Mode::Module]);
    }

    #[test]
    fn strict_runs_see_strict_code_both_ways() {
        for by in [StrictBy::Prefix, StrictBy::Opts] {
            assert_eq!(has_early_error("with (a) {}", Mode::Sloppy, by), Ok(None));
            assert!(
                has_early_error("with (a) {}", Mode::Strict, by)
                    .unwrap()
                    .is_some()
            );
            assert!(
                has_early_error("with (a) {}", Mode::Raw, by)
                    .unwrap()
                    .is_none()
            );
            assert!(
                has_early_error("with (a) {}", Mode::Module, by)
                    .unwrap()
                    .is_some()
            );
        }
    }

    #[test]
    fn pass_rules() {
        let runs = run_modes(
            "let a; let a;",
            &[Mode::Sloppy, Mode::Strict],
            true,
            StrictBy::Prefix,
        );
        assert!(runs.iter().all(|r| r.ok));
        let runs = run_modes("let a;", &[Mode::Sloppy], true, StrictBy::Prefix);
        assert!(!runs[0].ok);
        assert!(runs[0].detail.as_deref().unwrap().starts_with("parsed"));
        let runs = run_modes(
            "with (a) {}",
            &[Mode::Sloppy, Mode::Strict],
            false,
            StrictBy::Prefix,
        );
        assert!(runs[0].ok && !runs[1].ok);
    }

    #[test]
    fn infra_errors_are_not_parse_failures() {
        let outcome = |runs: Vec<Run>, error: Option<&str>| Outcome {
            rel: String::new(),
            module: false,
            negative: false,
            meta: None,
            runs,
            error: error.map(ToString::to_string),
        };
        // A parse mismatch is a failure that an expectation can list.
        let mismatch = outcome(
            run_modes("let a;", &[Mode::Sloppy], true, StrictBy::Prefix),
            None,
        );
        assert!(!mismatch.passed() && !mismatch.runs[0].panicked);
        assert_eq!(mismatch.infra_error(), None);
        // A read or front matter error, or a parser panic, is not.
        let unreadable = outcome(Vec::new(), Some("file is not valid UTF-8"));
        assert_eq!(
            unreadable.infra_error().as_deref(),
            Some("file is not valid UTF-8")
        );
        let panic = Run {
            mode: Mode::Strict,
            ok: false,
            panicked: true,
            detail: Some("panic: boom".into()),
        };
        assert_eq!(
            outcome(vec![panic], None).infra_error().as_deref(),
            Some("[strict] panic: boom")
        );
    }

    #[test]
    fn harness_needs_by_mode() {
        let outcome = |flags: &[&str], includes: &[&str], module: bool| Outcome {
            rel: String::new(),
            module,
            negative: false,
            meta: Some(Meta {
                includes: includes.iter().map(ToString::to_string).collect(),
                ..meta(flags)
            }),
            runs: Vec::new(),
            error: None,
        };
        let needs = include_needs(&[
            outcome(&["noStrict"], &["a.js"], false),
            outcome(&["module", "async"], &["b.js"], true),
            outcome(&["raw"], &["c.js"], false),
            outcome(&["onlyStrict"], &["a.js"], false),
        ]);
        assert_eq!(needs["a.js"], BTreeSet::from([Mode::Sloppy, Mode::Strict]));
        assert_eq!(needs["b.js"], BTreeSet::from([Mode::Sloppy]));
        assert_eq!(needs["doneprintHandle.js"], BTreeSet::from([Mode::Sloppy]));
        assert!(!needs.contains_key("c.js"));
        assert_eq!(
            needs["assert.js"],
            BTreeSet::from([Mode::Sloppy, Mode::Strict])
        );
    }
}
