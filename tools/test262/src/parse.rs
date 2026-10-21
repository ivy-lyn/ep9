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
//!
//! Both passes (tests, then includes) run in worker processes (`parse-worker`), as `exec` does,
//! so a crash or an abort loses one file, not the run:
//!
//! ```text
//! parent (parse)                                  parse-worker, one per thread
//!   "test\t<rel>" or "include\t<name>\t<modes>"  -> run_test / run_include
//!   <------------- one outcome line per request -- (rel, flags, includes, runs)
//!   worker exits (crash, abort): an infrastructure error with its stderr tail, respawn
//! ```
//!
//! A panic in a run is still caught there and is an infrastructure error. Under
//! `panic = "abort"` (P2-INFRA2) it ends the worker instead: also an infrastructure error, with
//! the panic text from the worker's stderr. Either way the run exits 2.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::{self, BufRead, Write as _};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ep9_frontend::{SourceOpts, Strictness, check_early_errors};

use crate::exec::{self, escape, unescape};
use crate::frontmatter::{self, Meta};
use crate::suite::{self, TestFile};
use crate::{expect, json};

/// The test directories of a parse run (09-milestones §8 week 1).
pub const DIRS: [&str; 4] = ["language", "built-ins", "annexB", "intl402"];

/// INTERPRETING.md "Strict Mode": this exact prefix.
pub(crate) const STRICT_PREFIX: &str = "\"use strict\";\n";

/// Worker stack. oxc has no depth limit yet (01-front-end §6), so give it room.
pub const STACK_BYTES: usize = 256 << 20;

/// Files a parent thread takes at a time (as `pool::par_map` does). It keeps up to two groups
/// in flight, and a worker writes its replies in groups of up to this many (or sooner when no
/// request waits), so neither side waits for a round trip per file. A file parses in about
/// 0.25 ms of CPU; one reply per file cost about 15% more CPU in a full run (macOS arm64).
const CHUNK: usize = 8;

/// A worker with no reply for this long is stuck: kill it (an infrastructure error). A file
/// parses in milliseconds.
const HARD_TIMEOUT: Duration = Duration::from_secs(120);

/// How strict runs become strict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrictBy {
    /// Prepend `"use strict";\n` to the source, as test262 says.
    Prefix,
    /// Pass `Strictness::Strict` to ep9_frontend and keep the source as-is.
    Opts,
}

impl StrictBy {
    fn name(self) -> &'static str {
        match self {
            StrictBy::Prefix => "prefix",
            StrictBy::Opts => "opts",
        }
    }

    pub fn from_name(s: &str) -> Option<StrictBy> {
        match s {
            "prefix" => Some(StrictBy::Prefix),
            "opts" => Some(StrictBy::Opts),
            _ => None,
        }
    }
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

    fn from_name(s: &str) -> Option<Mode> {
        match s {
            "sloppy" => Some(Mode::Sloppy),
            "strict" => Some(Mode::Strict),
            "raw" => Some(Mode::Raw),
            "module" => Some(Mode::Module),
            _ => None,
        }
    }
}

/// The flags of `parse-worker` (internal).
pub struct WorkerOptions {
    pub test262: PathBuf,
    pub strict_by: StrictBy,
}

#[derive(Debug, PartialEq, Eq)]
struct Run {
    mode: Mode,
    ok: bool,
    /// The parser panicked. This is not a parse result (see `Outcome::infra_error`).
    panicked: bool,
    /// Why the run failed.
    detail: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
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

/// Is a parse-side test hook set? (`EP9_TEST262_FAULT`; read once per process.)
fn faults_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("EP9_TEST262_FAULT").is_some())
}

/// Test hooks (only for tools/test262/tests) of a parse run of `name` (a test path or
/// `harness/<file>`), inside its `catch_unwind`: `parse-panic` panics; `parse-nounwind-panic`
/// panics where a panic cannot unwind, so the worker aborts as under `panic = "abort"`.
fn injected_parse_fault(name: &str) {
    if !faults_on() {
        return;
    }
    if exec::fault_for("parse-panic", name) {
        panic!("injected parse panic for {name}");
    }
    if exec::fault_for("parse-nounwind-panic", name) {
        exec::panic_without_unwind(format!("injected parse panic for {name}"));
    }
}

/// `Ok(Some(error))` = early error, `Ok(None)` = parsed, `Err(message)` = panic. `name` is the
/// file (for the test hooks).
fn has_early_error(
    src: &str,
    mode: Mode,
    strict_by: StrictBy,
    name: &str,
) -> Result<Option<String>, String> {
    let result = catch_unwind(AssertUnwindSafe(|| {
        injected_parse_fault(name);
        match mode {
            Mode::Sloppy | Mode::Raw => {
                check_early_errors(src, SourceOpts::script(Strictness::Sloppy))
            }
            Mode::Module => check_early_errors(src, SourceOpts::module()),
            Mode::Strict => match strict_by {
                StrictBy::Prefix => check_early_errors(
                    &format!("{STRICT_PREFIX}{src}"),
                    SourceOpts::script(Strictness::Sloppy),
                ),
                StrictBy::Opts => check_early_errors(src, SourceOpts::script(Strictness::Strict)),
            },
        }
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

fn run_modes(
    src: &str,
    modes: &[Mode],
    negative: bool,
    strict_by: StrictBy,
    name: &str,
) -> Vec<Run> {
    modes
        .iter()
        .map(|&mode| {
            let result = has_early_error(src, mode, strict_by, name);
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
    outcome.runs = run_modes(
        &src,
        &modes(&meta),
        outcome.negative,
        strict_by,
        &outcome.rel,
    );
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
            outcome.runs = run_modes(&src, &modes, false, strict_by, &outcome.rel);
        }
        Ok(Err(_)) => outcome.error = Some("file is not valid UTF-8".into()),
        Err(e) => outcome.error = Some(format!("cannot read: {e}")),
    }
    outcome
}

// ---------------------------------------------------------------------------------------------
// The parse worker and its line protocol

/// A request line: `test\t<rel>` or `include\t<name>\t<mode>,<mode>...` (fields escaped as in
/// the exec protocol).
fn test_request(rel: &str) -> String {
    format!("test\t{}", escape(rel))
}

fn include_request(name: &str, modes: &BTreeSet<Mode>) -> String {
    let modes: Vec<&str> = modes.iter().map(|m| m.name()).collect();
    format!("include\t{}\t{}", escape(name), modes.join(","))
}

/// One outcome as a line: rel, module, negative, error, meta present, flags, includes, then 4
/// fields per run (mode, ok, panicked, detail). Lists are joined by a newline before escaping;
/// an empty error or detail is none (a real one is never empty).
fn encode_outcome(o: &Outcome) -> String {
    let bit = |b: bool| if b { "1" } else { "0" };
    let (flags, includes) = o.meta.as_ref().map_or((String::new(), String::new()), |m| {
        (m.flags.join("\n"), m.includes.join("\n"))
    });
    let mut line = format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}",
        escape(&o.rel),
        bit(o.module),
        bit(o.negative),
        escape(o.error.as_deref().unwrap_or("")),
        bit(o.meta.is_some()),
        escape(&flags),
        escape(&includes)
    );
    for r in &o.runs {
        let _ = write!(
            line,
            "\t{}\t{}\t{}\t{}",
            r.mode.name(),
            bit(r.ok),
            bit(r.panicked),
            escape(r.detail.as_deref().unwrap_or(""))
        );
    }
    line
}

fn decode_outcome(line: &str) -> Option<Outcome> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() < 7 || !(fields.len() - 7).is_multiple_of(4) {
        return None;
    }
    let bit = |s: &str| match s {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    };
    let text = |s: &str| (!s.is_empty()).then(|| unescape(s));
    let list = |s: &str| -> Vec<String> {
        let s = unescape(s);
        if s.is_empty() {
            Vec::new()
        } else {
            s.split('\n').map(ToString::to_string).collect()
        }
    };
    let meta = bit(fields[4])?.then(|| Meta {
        flags: list(fields[5]),
        includes: list(fields[6]),
        ..Meta::default()
    });
    let runs = fields[7..]
        .chunks(4)
        .map(|r| {
            Some(Run {
                mode: Mode::from_name(r[0])?,
                ok: bit(r[1])?,
                panicked: bit(r[2])?,
                detail: text(r[3]),
            })
        })
        .collect::<Option<Vec<Run>>>()?;
    Some(Outcome {
        rel: unescape(fields[0]),
        module: bit(fields[1])?,
        negative: bit(fields[2])?,
        meta,
        runs,
        error: text(fields[3]),
    })
}

/// The outcome of a file whose worker crashed or hung on it: an infrastructure error.
fn gone_outcome(rel: String, gone: exec::Gone) -> Outcome {
    Outcome {
        rel,
        module: false,
        negative: false,
        meta: None,
        runs: Vec::new(),
        error: Some(format!("{}: {}", gone.cause, gone.detail)),
    }
}

/// Runs one request line in a worker. `None` = a bad request. `queued`: more requests wait
/// behind this one (for the test hook `parse-abort-in-group`).
fn serve(opts: &WorkerOptions, line: &str, queued: bool) -> Option<Outcome> {
    let mut fields = line.split('\t');
    let kind = fields.next()?;
    let name = unescape(fields.next()?);
    let outcome = match kind {
        "test" => {
            parse_abort_hook(&name, queued);
            let file = TestFile {
                path: opts.test262.join(&name),
                rel: name,
            };
            run_test(&file, opts.strict_by)
        }
        "include" => {
            parse_abort_hook(&format!("harness/{name}"), queued);
            let modes = fields
                .next()?
                .split(',')
                .map(Mode::from_name)
                .collect::<Option<BTreeSet<Mode>>>()?;
            run_include(&opts.test262, &name, &modes, opts.strict_by)
        }
        _ => return None,
    };
    fields.next().is_none().then_some(outcome)
}

/// Test hooks: `parse-abort` aborts the worker on a matching file (a crash in the parser).
/// `parse-abort-in-group` aborts only while more requests wait behind the file, so the crash
/// does not repeat when the file runs alone.
fn parse_abort_hook(name: &str, queued: bool) {
    if faults_on()
        && (exec::fault_for("parse-abort", name)
            || (queued && exec::fault_for("parse-abort-in-group", name)))
    {
        std::process::abort();
    }
}

/// `parse-worker`: reads request lines from stdin and writes one outcome line per request to
/// stdout, in order. Replies go out in groups of up to [`CHUNK`], or sooner when no request
/// waits. A crash loses the replies not written yet; the parent then runs those files again
/// one at a time (`exec::in_workers`), so the crash still lands on the right file.
pub fn worker(opts: WorkerOptions) -> ExitCode {
    exec::install_panic_hook();
    let runner = std::thread::Builder::new()
        .name("ep9-test262-parse".into())
        .stack_size(STACK_BYTES)
        .spawn(move || {
            let mut input = io::BufReader::new(io::stdin());
            let mut out = io::BufWriter::new(io::stdout().lock());
            let mut line = String::new();
            let mut unflushed = 0;
            loop {
                line.clear();
                match input.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(_) => return 2,
                }
                let request = line.trim_end_matches(['\n', '\r']);
                if request.is_empty() {
                    continue;
                }
                let queued = !input.buffer().is_empty();
                let Some(outcome) = serve(&opts, request, queued) else {
                    let _ = writeln!(
                        io::stderr(),
                        "ep9-test262 parse-worker: bad request {request:?}"
                    );
                    return 2;
                };
                if writeln!(out, "{}", encode_outcome(&outcome)).is_err() {
                    return 2;
                }
                unflushed += 1;
                // A full group, or nothing more to read: the parent may be waiting.
                if unflushed >= CHUNK || input.buffer().is_empty() {
                    if out.flush().is_err() {
                        return 2;
                    }
                    unflushed = 0;
                }
            }
            if out.flush().is_err() {
                return 2;
            }
            0
        });
    match runner.map(std::thread::JoinHandle::join) {
        Ok(Ok(code)) => ExitCode::from(code),
        _ => ExitCode::from(3),
    }
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

    let worker_args: Vec<String> = vec![
        "--test262".into(),
        root.display().to_string(),
        "--strict-by".into(),
        opts.strict_by.name().into(),
    ];
    let workers = exec::Workers {
        subcommand: "parse-worker",
        args: &worker_args,
        threads: opts.threads,
        chunk: CHUNK,
        hard: HARD_TIMEOUT,
    };
    let outcomes = exec::in_workers(
        &workers,
        &files,
        |f| test_request(&f.rel),
        |f, line| decode_outcome(line).filter(|o| o.rel == f.rel),
        |f, gone| gone_outcome(f.rel.clone(), gone),
        |_| {},
    );
    let Ok(outcomes) =
        outcomes.map_err(|e| eprintln!("ep9-test262: cannot start a worker process: {e}"))
    else {
        return ExitCode::from(2);
    };
    let needs: Vec<(String, BTreeSet<Mode>)> = include_needs(&outcomes).into_iter().collect();
    let includes = exec::in_workers(
        &workers,
        &needs,
        |(name, modes)| include_request(name, modes),
        |(name, _), line| decode_outcome(line).filter(|o| o.rel == format!("harness/{name}")),
        |(name, _), gone| gone_outcome(format!("harness/{name}"), gone),
        |_| {},
    );
    let Ok(includes) =
        includes.map_err(|e| eprintln!("ep9-test262: cannot start a worker process: {e}"))
    else {
        return ExitCode::from(2);
    };
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
            "ep9-test262: {} of {} files could not be checked (read error, bad front matter, parser \
             panic, worker crash or hang); the expectations were not compared or written",
            infra.len(),
            all().count()
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
            assert_eq!(
                has_early_error("with (a) {}", Mode::Sloppy, by, "t.js"),
                Ok(None)
            );
            assert!(
                has_early_error("with (a) {}", Mode::Strict, by, "t.js")
                    .unwrap()
                    .is_some()
            );
            assert!(
                has_early_error("with (a) {}", Mode::Raw, by, "t.js")
                    .unwrap()
                    .is_none()
            );
            assert!(
                has_early_error("with (a) {}", Mode::Module, by, "t.js")
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
            "t.js",
        );
        assert!(runs.iter().all(|r| r.ok));
        let runs = run_modes("let a;", &[Mode::Sloppy], true, StrictBy::Prefix, "t.js");
        assert!(!runs[0].ok);
        assert!(runs[0].detail.as_deref().unwrap().starts_with("parsed"));
        let runs = run_modes(
            "with (a) {}",
            &[Mode::Sloppy, Mode::Strict],
            false,
            StrictBy::Prefix,
            "t.js",
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
            run_modes("let a;", &[Mode::Sloppy], true, StrictBy::Prefix, "t.js"),
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

    #[test]
    fn outcome_lines_round_trip() {
        let test = Outcome {
            rel: "test/a\tb\\c.js".into(),
            module: true,
            negative: true,
            meta: Some(Meta {
                flags: vec!["module".into(), "async".into()],
                includes: vec!["x.js".into()],
                ..Meta::default()
            }),
            runs: vec![
                Run {
                    mode: Mode::Module,
                    ok: false,
                    panicked: true,
                    detail: Some("panic: a\nb\tc".into()),
                },
                Run {
                    mode: Mode::Strict,
                    ok: true,
                    panicked: false,
                    detail: None,
                },
            ],
            error: None,
        };
        let line = encode_outcome(&test);
        assert!(!line.contains('\n'), "{line}");
        assert_eq!(decode_outcome(&line).as_ref(), Some(&test));
        // No meta (an include), an error and no runs (an unreadable file), empty lists.
        let include = Outcome {
            rel: "harness/assert.js".into(),
            module: false,
            negative: false,
            meta: None,
            runs: Vec::new(),
            error: Some("cannot read: x".into()),
        };
        assert_eq!(
            decode_outcome(&encode_outcome(&include)).as_ref(),
            Some(&include)
        );
        let bare = Outcome {
            meta: Some(Meta::default()),
            error: None,
            ..include
        };
        assert_eq!(decode_outcome(&encode_outcome(&bare)).as_ref(), Some(&bare));
        // Bad lines.
        for bad in ["", "a\t1", "a\t1\t0\t\t0\t\t\tsloppy", "a\t2\t0\t\t0\t\t"] {
            assert_eq!(decode_outcome(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn requests_name_the_file_and_the_modes() {
        assert_eq!(test_request("test/a.js"), "test\ttest/a.js");
        let modes = BTreeSet::from([Mode::Strict, Mode::Sloppy]);
        assert_eq!(
            include_request("assert.js", &modes),
            "include\tassert.js\tsloppy,strict"
        );
        for m in [Mode::Sloppy, Mode::Strict, Mode::Raw, Mode::Module] {
            assert_eq!(Mode::from_name(m.name()), Some(m));
        }
        for by in [StrictBy::Prefix, StrictBy::Opts] {
            assert_eq!(StrictBy::from_name(by.name()), Some(by));
        }
    }

    #[test]
    fn a_worker_serves_tests_and_includes() {
        let root = std::env::temp_dir().join(format!("ep9-parse-serve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("harness")).unwrap();
        std::fs::create_dir_all(root.join("test")).unwrap();
        std::fs::write(root.join("harness/h.js"), "with (a) {}\n").unwrap();
        std::fs::write(
            root.join("test/t.js"),
            "/*---\nflags: [noStrict]\nincludes: [h.js]\n---*/\nvar a;\n",
        )
        .unwrap();
        let opts = WorkerOptions {
            test262: root.clone(),
            strict_by: StrictBy::Prefix,
        };
        let t = serve(&opts, "test\ttest/t.js", false).unwrap();
        assert!(t.passed());
        assert_eq!(t.meta.as_ref().unwrap().includes, ["h.js"]);
        let h = serve(&opts, "include\th.js\tsloppy,strict", false).unwrap();
        assert_eq!(h.rel, "harness/h.js");
        assert_eq!(h.runs.len(), 2);
        assert!(
            h.runs[0].ok && !h.runs[1].ok,
            "with is an early error only in strict code"
        );
        for bad in [
            "bogus\tx",
            "include\th.js",
            "include\th.js\tsloppy,bogus",
            "test\ta\tb",
        ] {
            assert_eq!(serve(&opts, bad, false), None, "{bad:?}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
