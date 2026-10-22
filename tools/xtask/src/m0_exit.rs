//! `cargo xtask m0-exit` (M0 exit check, docs/plan/09-milestones.md section 2.2): the table of
//! the 12 M0 exit items, PASS, FAIL, OPEN or NOT-RUN per item, for one host and for
//! `x86_64-pc-windows-msvc` from the jobs of a CI run, with the evidence of each cell.
//!
//! ```text
//! cargo xtask m0-exit --host <triple> --out m0-exit.md
//!     [--exec <json>]       ep9-test262 exec --json                     items 1, 3 (and 2, 3 on Windows)
//!     [--parse <json>]      ep9-test262 parse --json                    item 2
//!     [--exec-gc <json>]    ep9-test262 exec --gc-stress 1 ... --json   item 6
//!     [--cross <json>]      tools/bench/cross run (ep9-cross/v1)        item 4
//!     [--nesting <log>]     cargo test output of the nesting tests      item 7
//!     [--corpus <log>]...   cargo test output of the corpus digest test item 12 (release, debug)
//!     [--verdicts <md>]...  spike verdict tables (default: the newest
//!                           docs/measurements/*-m0-spike-review.md)     items 5, 11
//!     [--ci-run <id>] [--ci-run-json <file>]                            the Windows column
//!     [--no-build-check]    item 8 without `cargo check -p ep9_value`
//! ```
//!
//! The task reads its inputs; it runs no test suite. A missing input gives NOT-RUN with the
//! command that makes it. `--ci-run <id>` alone runs `gh run view <id> --json
//! jobs,headSha,databaseId,url`; `--ci-run-json <file>` reads a file in that shape instead (an
//! agent without gh, or a saved run). The Windows column reads NOT-RUN only when no run is given
//! or the run does not exist; items with no Windows job (4, 6) read NOT-RUN in it.
//!
//! | Item | Host column | Windows column (CI jobs `<job> (windows-x64)`) |
//! |---|---|---|
//! | 1 | exec JSON: full run (runs = the pin's count), wall time <= 600 s, 0 infra; the run names the committed expectation file of this tree, and its "fail" results against that file give 0 new failures and 0 stale lines (recomputed here, not the runner's diff) | step "test262 exec" green and <= 600 s; the file is committed |
//! | 2 | parse JSON: negatives all pass (the pin's count), positives >= 99.98% | step "test262 parse" green, and the Windows parse expectation file equals this host's |
//! | 3 | exec JSON: `language/` files >= 40% | step "test262 exec" green; passed = total (exec JSON) - the `test/language/` lines of the Windows expectation file |
//! | 4 | cross JSON: richards and deltablue vs QuickJS, net interval low end >= 0.6, valid rows (a gross fallback row, net invalid, gives OPEN), a gate host, not provisional | NOT-RUN (no Windows bench job) |
//! | 5 | verdicts: SP-GC, SP-VA, SP-Value, SP-Dispatch, SP-JIT-mem and Spike B GO (no open conditions); SP-tsrs-win recorded | the same verdicts, and these steps of the run green: SP-GC, SP-Dispatch, SP-JIT-mem and Spike B structural gates, ep9_heap barrier, cargo test --workspace, spike-native tests (ep9_vm) |
//! | 6 | GC JSON: the whole pinned corpus (files run + skipped = the pin's count; `skipped_paths` = the set of the committed skip list, and `skipped` = its size), `timeout_ms` <= 60000 and `feedback_threshold` 0 (GC_CMD), 0 infra; the run names the committed expectation file, and its "fail" results against that file without the skip list give 0 new failures (recomputed here) | NOT-RUN (CI runs GC-stress on linux-x64 only) |
//! | 7 | nesting log: `every_form_is_gated`, `caps_hold_end_to_end`, `uncaught_error_reports_file_line_utf16_column` ok, and no cargo failure line (`error[E`, `error:`, a panic) anywhere in it | step "cargo test --workspace" green (it runs the three) |
//! | 8 | the asserts are in ep9_value's source, and `cargo check -p ep9_value --target <host>` passes | step "cargo build --workspace --all-targets" green |
//! | 9, 10 | docs/adr/README.md index: the ADR status of every row due at M0 (Accepted or Frozen; a Draft for "Draft M0" rows); H20 Frozen | the same |
//! | 11 | verdicts: SP-Unit GO | the same |
//! | 12 | corpus logs: every run in every log (a run starts at a cargo "Finished" line) prints the digest of `expectations/bytecode-corpus.digest` and its test is ok, and no log has a cargo failure line (`error[E`, `error:`, a panic) anywhere; a release and a debug run are both given | step "bytecodegen corpus digest (release)" green |
//!
//! Exit 0 when every cell is PASS (the M0 exit holds), 1 otherwise, 2 on a usage error or an
//! input that cannot be read. The JSON reader is the small one below: xtask has no JSON crate.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The Windows triple; its column comes from CI.
const CI_TRIPLE: &str = "x86_64-pc-windows-msvc";
/// The CI matrix name of [`CI_TRIPLE`] (`.github/workflows/ci.yml`).
const CI_MATRIX: &str = "windows-x64";
/// Item 1: a full test262 run takes at most 10 minutes.
const MAX_WALL_SECONDS: f64 = 600.0;

/// Counts that depend on the test262 pin (09-milestones section 2.2 items 1 and 2).
struct PinCounts {
    pin_prefix: &'static str,
    /// Test files of a full exec run (item 6: run + skipped).
    files: u64,
    runs: u64,
    negatives: u64,
}

const PIN_COUNTS: &[PinCounts] = &[PinCounts {
    pin_prefix: "d53ebc8b",
    files: 53_616,
    runs: 102_994,
    negatives: 4_653,
}];

/// Item 10: the A0 part 1 ADRs.
const A0_PART1: &[&str] = &[
    "H2", "H3", "H5", "H7", "H8", "H13", "H14", "H27", "H35", "H36",
];

/// Item 7: the tests that check it (crates/ep9_frontend/src/depth_tests.rs,
/// crates/ep9_cli/tests/nesting_caps.rs, crates/ep9_cli/tests/cli.rs).
const NESTING_TESTS: &[&str] = &[
    "depth_tests::every_form_is_gated",
    "caps_hold_end_to_end",
    "uncaught_error_reports_file_line_utf16_column",
];
const NESTING_CMD: &str = "cargo test --locked -p ep9_frontend --lib depth_tests > nesting.log 2>&1; \
     cargo test --locked -p ep9_cli --test nesting_caps --test cli >> nesting.log 2>&1";

/// Item 8: the asserts that must be in crates/ep9_value/src/lib.rs.
const STATIC_ASSERTS: &[&str] = &[
    "size_of::<Value>() == 8",
    "size_of::<JsResult<Value>>() == 8",
    "size_of::<Word0>() == 8",
];

/// Item 5: the spikes that must be GO, and the one that must be recorded.
const ITEM5_SPIKES: &[&str] = &[
    "sp-gc",
    "sp-va",
    "sp-value",
    "sp-dispatch",
    "sp-jit-mem",
    "spike b",
];

/// Cell status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Status {
    Pass,
    Open,
    NotRun,
    Fail,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
            Status::Open => "OPEN",
            Status::NotRun => "NOT-RUN",
        }
    }
}

/// One cell: a status and why.
#[derive(Clone, Debug)]
struct Cell {
    status: Status,
    evidence: String,
}

fn cell(status: Status, evidence: impl Into<String>) -> Cell {
    Cell {
        status,
        evidence: evidence.into(),
    }
}

/// The parsed command line.
#[derive(Debug, Default)]
struct Options {
    root: PathBuf,
    host: String,
    out: PathBuf,
    exec: Option<PathBuf>,
    parse: Option<PathBuf>,
    exec_gc: Option<PathBuf>,
    cross: Option<PathBuf>,
    nesting: Option<PathBuf>,
    corpus: Vec<PathBuf>,
    verdicts: Vec<PathBuf>,
    ci_run: Option<String>,
    ci_run_json: Option<PathBuf>,
    build_check: bool,
}

const USAGE: &str = "usage: cargo xtask m0-exit --host <triple> --out <file.md> [--exec <json>] \
     [--parse <json>] [--exec-gc <json>] [--cross <json>] [--nesting <log>] [--corpus <log>]... \
     [--verdicts <md>]... [--ci-run <id>] [--ci-run-json <file>] [--no-build-check]";

fn parse_args(args: &[String], root: PathBuf) -> Result<Options, String> {
    let mut o = Options {
        root,
        build_check: true,
        ..Options::default()
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{a} needs a value\n{USAGE}"))
        };
        match a.as_str() {
            "--host" => o.host = val()?,
            "--out" => o.out = PathBuf::from(val()?),
            "--exec" => o.exec = Some(PathBuf::from(val()?)),
            "--parse" => o.parse = Some(PathBuf::from(val()?)),
            "--exec-gc" => o.exec_gc = Some(PathBuf::from(val()?)),
            "--cross" => o.cross = Some(PathBuf::from(val()?)),
            "--nesting" => o.nesting = Some(PathBuf::from(val()?)),
            "--corpus" => o.corpus.push(PathBuf::from(val()?)),
            "--verdicts" => o.verdicts.push(PathBuf::from(val()?)),
            "--ci-run" => o.ci_run = Some(val()?),
            "--ci-run-json" => o.ci_run_json = Some(PathBuf::from(val()?)),
            "--no-build-check" => o.build_check = false,
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }
    if o.host.is_empty() || o.out.as_os_str().is_empty() {
        return Err(format!("--host and --out are required\n{USAGE}"));
    }
    if let Some(id) = &o.ci_run
        && (id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(format!("--ci-run needs a numeric run id, not {id:?}"));
    }
    Ok(o)
}

/// Entry point: writes the report to `--out` and prints the table.
pub fn main(args: &[String]) -> Result<bool, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    let opts = parse_args(args, root)?;
    let report = run(&opts)?;
    std::fs::write(&opts.out, &report.markdown)
        .map_err(|e| format!("cannot write {}: {e}", opts.out.display()))?;
    print!("{}", report.table);
    println!("m0-exit: wrote {}", opts.out.display());
    Ok(report.all_pass())
}

/// The rendered result.
#[derive(Debug)]
struct Report {
    table: String,
    markdown: String,
    /// Per item (1-12): the host cell and the Windows cell.
    cells: Vec<(Cell, Cell)>,
}

impl Report {
    /// Every cell is PASS: the M0 exit holds.
    fn all_pass(&self) -> bool {
        self.cells
            .iter()
            .all(|(h, w)| h.status == Status::Pass && w.status == Status::Pass)
    }
}

/// Everything the items read, loaded once.
struct Ctx<'a> {
    o: &'a Options,
    pin: Option<String>,
    pin_counts: Option<&'static PinCounts>,
    exec: Option<(PathBuf, Json)>,
    parse: Option<(PathBuf, Json)>,
    gc: Option<(PathBuf, Json)>,
    cross: Option<(PathBuf, Json)>,
    nesting: Option<(PathBuf, String)>,
    corpus: Vec<(PathBuf, String)>,
    verdicts: BTreeMap<String, (String, PathBuf)>,
    adr: Vec<AdrRow>,
    ci: Result<Ci, String>,
}

fn run(o: &Options) -> Result<Report, String> {
    let read_json = |p: &Option<PathBuf>| -> Result<Option<(PathBuf, Json)>, String> {
        p.as_ref()
            .map(|p| {
                Ok((
                    p.clone(),
                    parse_json(&read(p)?).map_err(|e| format!("{}: {e}", p.display()))?,
                ))
            })
            .transpose()
    };
    let pin = std::fs::read_to_string(o.root.join("third_party/test262.commit"))
        .ok()
        .map(|s| s.trim().to_string());
    let pin_counts = pin
        .as_deref()
        .and_then(|p| PIN_COUNTS.iter().find(|c| p.starts_with(c.pin_prefix)));
    let mut verdict_files = o.verdicts.clone();
    if let Some(review) = newest_spike_review(&o.root) {
        verdict_files.insert(0, review);
    }
    let mut verdicts = BTreeMap::new();
    for f in &verdict_files {
        for (spike, v) in verdict_table(&read(f)?) {
            verdicts.insert(spike, (v, f.clone()));
        }
    }
    let ci = match (&o.ci_run, &o.ci_run_json) {
        (_, Some(file)) => {
            let ci = Ci::from_json(
                &parse_json(&read(file)?).map_err(|e| format!("{}: {e}", file.display()))?,
            )?;
            if let (Some(want), Some(got)) = (&o.ci_run, &ci.id)
                && want != got
            {
                return Err(format!(
                    "--ci-run {want} but {} holds run {got}",
                    file.display()
                ));
            }
            Ok(ci)
        }
        (Some(id), None) => gh_run(id),
        (None, None) => Err("no CI run given (--ci-run <id> or --ci-run-json <file>)".into()),
    };
    let ctx = Ctx {
        o,
        pin,
        pin_counts,
        exec: read_json(&o.exec)?,
        parse: read_json(&o.parse)?,
        gc: read_json(&o.exec_gc)?,
        cross: read_json(&o.cross)?,
        nesting: o
            .nesting
            .as_ref()
            .map(|p| Ok::<_, String>((p.clone(), read(p)?)))
            .transpose()?,
        corpus: o
            .corpus
            .iter()
            .map(|p| Ok((p.clone(), read(p)?)))
            .collect::<Result<_, String>>()?,
        verdicts,
        adr: adr_index(&read(&o.root.join("docs/adr/README.md"))?)?,
        ci,
    };
    for (what, j) in [("--exec", &ctx.exec), ("--exec-gc", &ctx.gc)] {
        if let Some((p, j)) = j
            && j.get("command").and_then(Json::str) != Some("exec")
        {
            return Err(format!(
                "{what} {}: not an ep9-test262 exec JSON",
                p.display()
            ));
        }
    }
    if let Some((p, j)) = &ctx.parse
        && j.get("command").and_then(Json::str) != Some("parse")
    {
        return Err(format!(
            "--parse {}: not an ep9-test262 parse JSON",
            p.display()
        ));
    }
    for (p, j) in [&ctx.exec, &ctx.gc, &ctx.parse].into_iter().flatten() {
        let t = j.get("target").and_then(Json::str).unwrap_or("");
        if t != o.host {
            return Err(format!(
                "{}: target {t:?}, but --host is {:?}",
                p.display(),
                o.host
            ));
        }
    }

    type ItemFn = fn(&Ctx) -> (Cell, Cell);
    let items: [(&str, ItemFn); 12] = [
        (
            "Full test262 run <= 10 min; expectations committed per target",
            item1,
        ),
        ("Parse negatives all pass; positives >= 99.98%", item2),
        ("`language/` >= 40%", item3),
        ("richards and deltablue run, each >= 0.6x QuickJS", item4),
        (
            "Spikes green (SP-GC, SP-VA, SP-Value, SP-Dispatch, SP-JIT-mem, Spike B); SP-tsrs-win recorded",
            item5,
        ),
        ("GC-stress passes every passing test", item6),
        (
            "Nesting forms: cap-1 compiles, cap+1 RangeError; file:line:col with a UTF-16 column",
            item7,
        ),
        ("Static asserts: Value, JsResult and word0 are 8 B", item8),
        (
            "Every \"Fix by M0\" H row merged as an ADR; builtin ABI frozen",
            item9,
        ),
        ("A0 part 1 ADRs merged", item10),
        ("SP-Unit green on 4 targets", item11),
        (
            "Identical bytecode hashes on the test262 + bench corpus",
            item12,
        ),
    ];
    let cells: Vec<(Cell, Cell)> = items.iter().map(|(_, f)| f(&ctx)).collect();
    Ok(render(&ctx, &items.map(|(t, _)| t), cells))
}

fn render(ctx: &Ctx, titles: &[&str; 12], cells: Vec<(Cell, Cell)>) -> Report {
    let o = ctx.o;
    let head = git(&o.root, &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let mut table = String::new();
    let _ = writeln!(
        table,
        "| # | M0 exit item | {} | {CI_TRIPLE} (CI) |",
        o.host
    );
    let _ = writeln!(table, "|---|---|---|---|");
    for (i, (t, (h, w))) in titles.iter().zip(&cells).enumerate() {
        let _ = writeln!(
            table,
            "| {} | {t} | {} | {} |",
            i + 1,
            h.status.name(),
            w.status.name()
        );
    }
    let all_pass = cells
        .iter()
        .all(|(h, w)| h.status == Status::Pass && w.status == Status::Pass);
    let mut md = String::from("# M0 exit check (`cargo xtask m0-exit`)\n\n");
    let _ = writeln!(
        md,
        "- Items: docs/plan/09-milestones.md section 2.2, M0 exit 1-12."
    );
    let _ = writeln!(md, "- Host column: {}; tree at commit `{head}`.", o.host);
    let _ = writeln!(
        md,
        "- test262 pin: `{}`.",
        ctx.pin.as_deref().unwrap_or("unknown")
    );
    match &ctx.ci {
        Ok(ci) => {
            let _ = writeln!(
                md,
                "- {CI_TRIPLE} column: CI run {} on commit `{}`{} ({} jobs).",
                ci.id.as_deref().unwrap_or("(no id)"),
                ci.head.as_deref().unwrap_or("unknown"),
                ci.url
                    .as_deref()
                    .map(|u| format!(", {u}"))
                    .unwrap_or_default(),
                ci.jobs.len()
            );
        }
        Err(e) => {
            let _ = writeln!(md, "- {CI_TRIPLE} column: NOT-RUN: {e}.");
        }
    }
    let _ = writeln!(
        md,
        "- Result: {}.\n",
        if all_pass {
            "every item PASS on both columns: the M0 exit holds"
        } else {
            "not every cell is PASS"
        }
    );
    md.push_str(&table);
    md.push_str("\n## Evidence\n");
    for (i, (t, (h, w))) in titles.iter().zip(&cells).enumerate() {
        let _ = write!(
            md,
            "\n### {}. {t}\n\n- {} {}: {}\n- {CI_TRIPLE} {}: {}\n",
            i + 1,
            o.host,
            h.status.name(),
            h.evidence,
            w.status.name(),
            w.evidence
        );
    }
    Report {
        table,
        markdown: md,
        cells,
    }
}

// ---------------------------------------------------------------------------------------------
// Items.

fn not_given(flag: &str, how: &str) -> Cell {
    cell(
        Status::NotRun,
        format!("no {flag} input; make it with `{how}`"),
    )
}

const EXEC_CMD: &str = "cargo run --release --locked -p ep9_test262 -- exec --json exec.json";

/// The committed expectation file `expectations/<name>`: Ok(path) if it exists and git tracks it.
fn committed(root: &Path, name: &str) -> Result<PathBuf, String> {
    let p = root.join("expectations").join(name);
    if !p.is_file() {
        return Err(format!("expectations/{name} does not exist"));
    }
    match Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(&p)
        .output()
    {
        Ok(out) if out.status.success() => Ok(p),
        Ok(out) if String::from_utf8_lossy(&out.stderr).contains("not a git repository") => Ok(p),
        Ok(_) => Err(format!("expectations/{name} is not committed")),
        Err(_) => Ok(p), // no git: the file exists
    }
}

/// The paths of an expectation file (comments and blank lines dropped).
fn expectation_set(p: &Path) -> Result<BTreeSet<String>, String> {
    Ok(read(p)?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect())
}

fn count(j: &Json, path: &[&str]) -> Option<(u64, u64)> {
    let c = j.path(path)?;
    Some((
        c.get("passed")?.num()? as u64,
        c.get("total")?.num()? as u64,
    ))
}

fn pct(p: u64, t: u64) -> f64 {
    if t == 0 {
        0.0
    } else {
        100.0 * p as f64 / t as f64
    }
}

/// Infra results and the expectation diff of an exec JSON.
fn exec_health(j: &Json) -> (usize, usize, usize) {
    let infra = j.get("results").and_then(Json::arr).map_or(0, |r| {
        r.iter()
            .filter(|r| r.get("status").and_then(Json::str) == Some("infra"))
            .count()
    });
    let len = |k: &str| {
        j.path(&["expectations", k])
            .and_then(Json::arr)
            .map_or(0, <[Json]>::len)
    };
    (infra, len("new_failures"), len("now_passing"))
}

/// The expectation diff of an exec JSON, recomputed here: its "fail" results against the
/// committed `expectations/<file>` without `skip` (the tests the run did not run). The runner's
/// own diff is not enough: it stays empty when the runner cannot read its expectation file, and
/// `--expect` can name any file. So the run must name the committed file of this tree.
#[derive(Debug)]
struct Recheck {
    /// Failing, and not in the file.
    new: Vec<String>,
    /// In the file, and not failing.
    stale: Vec<String>,
}

fn recheck(root: &Path, j: &Json, file: &str, skip: &BTreeSet<String>) -> Result<Recheck, String> {
    let want = committed(root, file)?;
    let used = j
        .path(&["expectations", "file"])
        .and_then(Json::str)
        .unwrap_or("");
    let same = match (std::fs::canonicalize(used), std::fs::canonicalize(&want)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same {
        return Err(format!(
            "the run compared with {used:?}, not expectations/{file} of this tree ({}){}",
            want.display(),
            if Path::new(used).is_file() {
                ""
            } else {
                ": that file does not exist, so the runner could not read it and compared nothing"
            }
        ));
    }
    let expected: BTreeSet<String> = expectation_set(&want)?.difference(skip).cloned().collect();
    let results = j
        .get("results")
        .and_then(Json::arr)
        .ok_or("the run has no \"results\" array")?;
    let failing: BTreeSet<String> = results
        .iter()
        .filter(|r| r.get("status").and_then(Json::str) == Some("fail"))
        .filter_map(|r| r.get("path").and_then(Json::str).map(str::to_string))
        .collect();
    Ok(Recheck {
        new: failing.difference(&expected).cloned().collect(),
        stale: expected.difference(&failing).cloned().collect(),
    })
}

/// Up to 5 paths, for the evidence.
fn some_paths(paths: &[String]) -> String {
    let mut t = paths.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
    if paths.len() > 5 {
        let _ = write!(t, " and {} more", paths.len() - 5);
    }
    t
}

fn item1(c: &Ctx) -> (Cell, Cell) {
    let file = format!("test262.{}.jit.txt", c.o.host);
    let host = match &c.exec {
        None => not_given("--exec", EXEC_CMD),
        Some((p, j)) => {
            let runs = count(j, &["runs"]);
            let wall = j
                .get("wall_seconds")
                .and_then(Json::num)
                .unwrap_or(f64::INFINITY);
            let gc = j.get("gc_stress").and_then(Json::num).unwrap_or(0.0);
            let (infra, runner_new, runner_stale) = exec_health(j);
            let mut why = Vec::new();
            let mut status = Status::Pass;
            let (ok_runs, total_runs) = runs.unwrap_or((0, 0));
            match c.pin_counts {
                Some(pc) if total_runs != pc.runs => {
                    status = Status::Fail;
                    why.push(format!(
                        "{total_runs} runs, not the full {} of the pin (a filtered run?)",
                        pc.runs
                    ));
                }
                Some(_) => {}
                None => {
                    status = status.max(Status::Open);
                    why.push(format!(
                        "no full-run count for the pin {} in m0_exit.rs PIN_COUNTS",
                        c.pin.as_deref().unwrap_or("unknown")
                    ));
                }
            }
            if gc != 0.0 {
                status = Status::Fail;
                why.push(format!(
                    "a GC-stress run (--gc-stress {gc}); give the default run"
                ));
            }
            if wall > MAX_WALL_SECONDS {
                status = Status::Fail;
                why.push(format!("wall time {wall:.1} s > {MAX_WALL_SECONDS} s"));
            }
            // The diff against the committed file, recomputed; the runner's diff counts too.
            let mut basis = format!("recomputed against expectations/{file}");
            let (new, stale) = match recheck(&c.o.root, j, &file, &BTreeSet::new()) {
                Ok(d) => {
                    if !d.new.is_empty() {
                        why.push(format!("new failures: {}", some_paths(&d.new)));
                    }
                    (d.new.len().max(runner_new), d.stale.len().max(runner_stale))
                }
                Err(e) => {
                    status = Status::Fail;
                    why.push(e);
                    basis = "the runner's diff, not recomputed".into();
                    (runner_new, runner_stale)
                }
            };
            if infra > 0 || new > 0 {
                status = Status::Fail;
                why.push(format!("{infra} infrastructure errors, {new} new failures"));
            }
            if stale > 0 {
                status = status.max(Status::Open);
                why.push(format!(
                    "{stale} expectation lines now pass (regenerate the file)"
                ));
            }
            cell(
                status,
                format!(
                    "{}: {ok_runs} / {total_runs} runs ok, wall time {wall:.1} s (<= {MAX_WALL_SECONDS} s), {} workers, infra {infra}, new failures {new}, stale lines {stale} ({basis}){}",
                    p.display(),
                    j.get("workers").and_then(Json::num).unwrap_or(0.0),
                    if why.is_empty() {
                        String::new()
                    } else {
                        format!(". {}", why.join("; "))
                    }
                ),
            )
        }
    };
    let win_file = format!("test262.{CI_TRIPLE}.jit.txt");
    let win = with_ci(c, |ci| {
        let s = ci.step("test262 parse + exec", &["test262 exec"])?;
        let secs = s.seconds();
        let mut status = s.status();
        let mut text = format!("step \"test262 exec\" {}", s.describe());
        if status == Status::Pass {
            match secs {
                Some(t) if t as f64 > MAX_WALL_SECONDS => {
                    status = Status::Fail;
                    let _ = write!(text, "; {t} s > {MAX_WALL_SECONDS} s");
                }
                Some(t) => {
                    let _ = write!(
                        text,
                        " in {t} s (step time, <= {MAX_WALL_SECONDS} s; a full run against expectations/{win_file})"
                    );
                }
                None => {
                    status = Status::Open;
                    text.push_str("; the step has no start or end time");
                }
            }
        }
        if let Err(e) = committed(&c.o.root, &win_file) {
            status = Status::Fail;
            let _ = write!(text, "; {e}");
        }
        Ok(cell(status, text))
    });
    (host, win)
}

const PARSE_CMD: &str = "cargo run --release --locked -p ep9_test262 -- parse --json parse.json";

fn item2(c: &Ctx) -> (Cell, Cell) {
    let host = match &c.parse {
        None => not_given("--parse", PARSE_CMD),
        Some((p, j)) => {
            let (np, nt) = count(j, &["negatives", "all"]).unwrap_or((0, 0));
            let (pp, pt) = count(j, &["positives", "all"]).unwrap_or((0, 0));
            let mut status = Status::Pass;
            let mut why = Vec::new();
            if np != nt || nt == 0 {
                status = Status::Fail;
                why.push(format!("{} negatives fail", nt - np));
            }
            match c.pin_counts {
                Some(pc) if nt != pc.negatives => {
                    status = Status::Fail;
                    why.push(format!(
                        "{nt} negatives, not the {} of the pin (a filtered run?)",
                        pc.negatives
                    ));
                }
                Some(_) => {}
                None => {
                    status = status.max(Status::Open);
                    why.push("no negative count for this pin in PIN_COUNTS".into());
                }
            }
            // >= 99.98%, in integers: passed * 10000 >= total * 9998.
            if pt == 0 || pp * 10_000 < pt * 9_998 {
                status = Status::Fail;
                why.push(format!("positives {:.3}% < 99.98%", pct(pp, pt)));
            }
            cell(
                status,
                format!(
                    "{}: negatives {np} / {nt}, positives {pp} / {pt} ({:.3}%){}",
                    p.display(),
                    pct(pp, pt),
                    if why.is_empty() {
                        String::new()
                    } else {
                        format!(". {}", why.join("; "))
                    }
                ),
            )
        }
    };
    let win = with_ci(c, |ci| {
        let s = ci.step("test262 parse + exec", &["test262 parse"])?;
        let mut text = format!("step \"test262 parse\" {}", s.describe());
        if s.status() != Status::Pass {
            return Ok(cell(s.status(), text));
        }
        let mine =
            c.o.root
                .join(format!("expectations/test262-parse.{}.txt", c.o.host));
        let theirs =
            c.o.root
                .join(format!("expectations/test262-parse.{CI_TRIPLE}.txt"));
        let (a, b) = (expectation_set(&mine)?, expectation_set(&theirs)?);
        if a != b {
            let _ = write!(
                text,
                "; its expectation file differs from {}'s ({} paths differ), so this host's counts do not carry over",
                c.o.host,
                a.symmetric_difference(&b).count()
            );
            return Ok(cell(Status::Open, text));
        }
        let _ = write!(
            text,
            " (the run equals expectations/test262-parse.{CI_TRIPLE}.txt, which lists the same {} failures as {}'s)",
            a.len(),
            c.o.host
        );
        Ok(match &host {
            h if h.status == Status::Pass => cell(
                Status::Pass,
                format!("{text}; so the host counts hold: {}", h.evidence),
            ),
            h => cell(
                Status::Open,
                format!("{text}; the host counts are not PASS ({})", h.status.name()),
            ),
        })
    });
    (host, win)
}

fn item3(c: &Ctx) -> (Cell, Cell) {
    let lang = c
        .exec
        .as_ref()
        .and_then(|(_, j)| count(j, &["by_dir", "language", "files"]));
    let host = match (&c.exec, lang) {
        (None, _) => not_given("--exec", EXEC_CMD),
        (Some((p, _)), None) => cell(
            Status::Fail,
            format!("{}: no by_dir.language counts", p.display()),
        ),
        (Some((p, _)), Some((lp, lt))) => cell(
            if lp * 100 >= lt * 40 && lt > 0 {
                Status::Pass
            } else {
                Status::Fail
            },
            format!(
                "{}: language/ {lp} / {lt} files = {:.2}% (>= 40%)",
                p.display(),
                pct(lp, lt)
            ),
        ),
    };
    let win = with_ci(c, |ci| {
        let s = ci.step("test262 parse + exec", &["test262 exec"])?;
        let text = format!("step \"test262 exec\" {}", s.describe());
        if s.status() != Status::Pass {
            return Ok(cell(s.status(), text));
        }
        let Some((_, lt)) = lang else {
            return Ok(cell(
                Status::Open,
                format!("{text}; the language/ total needs --exec"),
            ));
        };
        let fails = expectation_set(
            &c.o.root
                .join(format!("expectations/test262.{CI_TRIPLE}.jit.txt")),
        )?
        .iter()
        .filter(|p| p.starts_with("test/language/"))
        .count() as u64;
        let lp = lt.saturating_sub(fails);
        Ok(cell(
            if lp * 100 >= lt * 40 && lt > 0 {
                Status::Pass
            } else {
                Status::Fail
            },
            format!(
                "{text}, so its failures are expectations/test262.{CI_TRIPLE}.jit.txt: language/ {lp} / {lt} files = {:.2}% (total from the host exec JSON; {fails} listed failures)",
                pct(lp, lt)
            ),
        ))
    });
    (host, win)
}

fn item4(c: &Ctx) -> (Cell, Cell) {
    let host = match &c.cross {
        None => not_given(
            "--cross",
            "node tools/bench/cross/run.mjs ... --out run.json (a gate-host run; or a committed docs/measurements/*.json)",
        ),
        Some((p, j)) => cross_canary(p, j, &c.o.root),
    };
    let win = cell(
        Status::NotRun,
        "no CI job runs richards or deltablue on windows-x64 (the canary is a gate-host speed reading, 10-quality section 3)",
    );
    (host, win)
}

fn cross_canary(p: &Path, j: &Json, root: &Path) -> Cell {
    if j.get("schema").and_then(Json::str) != Some("ep9-cross/v1") {
        return cell(
            Status::Fail,
            format!("{}: not an ep9-cross/v1 run", p.display()),
        );
    }
    let commit = j
        .path(&["commit", "ep9"])
        .and_then(Json::str)
        .unwrap_or("unknown");
    let head = git(root, &["rev-parse", "HEAD"]).unwrap_or_default();
    let provisional = j.get("provisional").and_then(Json::bool).unwrap_or(true);
    let gate_host = j
        .path(&["host", "gate_host"])
        .and_then(Json::bool)
        .unwrap_or(false);
    let rows = j.path(&["M", "ratios"]).and_then(Json::arr).unwrap_or(&[]);
    let mut status = Status::Pass;
    let mut parts = Vec::new();
    for w in ["richards", "deltablue"] {
        let row = rows.iter().find(|r| {
            r.get("workload").and_then(Json::str) == Some(w)
                && r.get("comparator").and_then(Json::str) == Some("qjs")
        });
        let Some(row) = row else {
            status = Status::Fail;
            parts.push(format!("{w}: no row vs QuickJS (did it run?)"));
            continue;
        };
        // The gate reads net ratios only (run.mjs label()): a gross fallback row, written when
        // net time is <= 0 in a round, passes or fails nothing.
        let metric = row.get("metric").and_then(Json::str).unwrap_or("");
        let invalid = row.get("net_invalid").and_then(Json::str);
        let Some(net) = row.get("net").filter(|n| **n != Json::Null) else {
            status = status.max(Status::Open);
            parts.push(format!(
                "{w}: net invalid ({}): no gate verdict (a {metric} ratio is shown, not gated)",
                invalid.unwrap_or("no net ratio")
            ));
            continue;
        };
        if metric != "net" || invalid.is_some() {
            status = status.max(Status::Open);
            parts.push(format!(
                "{w}: metric {metric:?}, net invalid ({}): no gate verdict",
                invalid.unwrap_or("none given")
            ));
            continue;
        }
        let median = net.get("median").and_then(Json::num).unwrap_or(0.0);
        let low = net
            .get("interval")
            .and_then(Json::arr)
            .and_then(|a| a.first())
            .and_then(Json::num)
            .unwrap_or(0.0);
        let void = net
            .get("void")
            .or_else(|| row.get("void"))
            .and_then(Json::bool)
            .unwrap_or(true);
        if low < 0.6 {
            status = Status::Fail;
        } else if void {
            status = status.max(Status::Open);
        }
        parts.push(format!(
            "{w} net {median:.3}x (interval low end {low:.3}{})",
            if void { ", void row" } else { "" }
        ));
    }
    if provisional || !gate_host {
        status = status.max(Status::Open);
        parts.push(format!("provisional {provisional}, gate host {gate_host}"));
    }
    cell(
        status,
        format!(
            "{}: {}; measured at ep9 commit `{}`{}",
            p.display(),
            parts.join("; "),
            short(commit),
            if !head.is_empty() && (head.starts_with(commit) || commit.starts_with(&head)) {
                String::new()
            } else {
                format!(
                    " (not this tree, `{}`: the canary is the newest gate-host run given)",
                    short(&head)
                )
            }
        ),
    )
}

/// The verdict of one spike: (status, text).
fn spike_status(c: &Ctx, spike: &str, recorded_ok: bool) -> (Status, String) {
    let Some((v, file)) = c.verdicts.get(spike) else {
        return (Status::Open, format!("{spike}: no verdict"));
    };
    let l = v.to_ascii_lowercase();
    let status = if l.contains("no-go") || l.contains("fail") {
        Status::Fail
    } else if (l.starts_with("go") && !l.contains("condition"))
        || (recorded_ok && l.starts_with("recorded"))
    {
        Status::Pass
    } else {
        Status::Open
    };
    (
        status,
        format!(
            "{spike}: {v} ({})",
            file.file_name()
                .map_or(String::new(), |f| f.to_string_lossy().into_owned())
        ),
    )
}

fn item5(c: &Ctx) -> (Cell, Cell) {
    let mut status = Status::Pass;
    let mut parts = Vec::new();
    for s in ITEM5_SPIKES {
        let (st, t) = spike_status(c, s, false);
        status = status.max(st);
        parts.push(t);
    }
    let (st, t) = spike_status(c, "sp-tsrs-win", true);
    status = status.max(st);
    parts.push(t);
    let host = cell(status, parts.join("; "));
    let win = with_ci(c, |ci| {
        let mut st = Status::Pass;
        let mut parts = Vec::new();
        for (job, step) in [
            ("spike gates", "SP-GC gate"),
            ("spike gates", "SP-Dispatch gate"),
            ("spike gates", "SP-JIT-mem gate"),
            ("spike gates", "Spike B structural gate"),
            ("disasm checks", "ep9_heap barrier"),
            ("build + test", "Run cargo test --workspace --locked"),
            // D21: the workspace test does not enable spike-native; this step is the only
            // Windows run of Spike B's native code.
            ("build + test", "spike-native tests (ep9_vm)"),
        ] {
            match ci.step(job, &[step]) {
                Ok(s) => {
                    st = st.max(s.status());
                    parts.push(format!("\"{step}\" {}", s.describe()));
                }
                Err(e) => {
                    st = st.max(Status::NotRun);
                    parts.push(e);
                }
            }
        }
        let text = format!(
            "CI: {} (SP-VA runs in the test step, SP-Value in the barrier step); verdicts: {}",
            parts.join(", "),
            host.evidence
        );
        Ok(cell(st.max(host.status), text))
    });
    (host, win)
}

const GC_CMD: &str = "cargo run --release --locked -p ep9_test262 -- exec --gc-stress 1 --feedback-threshold 0 --timeout 60 --skip-list expectations/test262-gc-skip.txt --json gc.json";
/// Item 6: the `--timeout` of [`GC_CMD`], in ms (a shorter one is stricter).
const GC_TIMEOUT_MS: f64 = 60_000.0;

fn item6(c: &Ctx) -> (Cell, Cell) {
    let host = match &c.gc {
        None => not_given("--exec-gc", GC_CMD),
        Some((p, j)) => {
            let gc = j.get("gc_stress").and_then(Json::num).unwrap_or(0.0);
            let (infra, runner_new, runner_stale) = exec_health(j);
            let skipped = j.get("skipped").and_then(Json::num).unwrap_or(0.0) as usize;
            // The skipped tests by path (an older runner wrote only their count).
            let skipped_paths: Option<BTreeSet<String>> =
                j.get("skipped_paths").and_then(Json::arr).map(|a| {
                    a.iter()
                        .filter_map(|p| p.str().map(str::to_string))
                        .collect()
                });
            let (fp, ft) = count(j, &["files"]).unwrap_or((0, 0));
            let mut status = Status::Pass;
            let mut why = Vec::new();
            if gc <= 0.0 {
                status = Status::Fail;
                why.push("not a GC-stress run".to_string());
            }
            // The timeout and the feedback threshold of GC_CMD: a longer timeout lets a timeout
            // skip pass, and lazy feedback is another run.
            match j.get("timeout_ms").and_then(Json::num) {
                Some(t) if t <= GC_TIMEOUT_MS => {}
                t => {
                    status = Status::Fail;
                    why.push(format!(
                        "timeout_ms {}, not at most {GC_TIMEOUT_MS} (--timeout 60)",
                        t.map_or("missing".to_string(), |t| t.to_string())
                    ));
                }
            }
            match j.get("feedback_threshold").and_then(Json::num) {
                Some(0.0) => {}
                f => {
                    status = Status::Fail;
                    why.push(format!(
                        "feedback_threshold {}, not 0 (--feedback-threshold 0)",
                        f.map_or("missing".to_string(), |f| f.to_string())
                    ));
                }
            }
            // The whole pinned corpus: every file is run or skipped by the skip list.
            match c.pin_counts {
                Some(pc) if ft + skipped as u64 != pc.files => {
                    status = Status::Fail;
                    why.push(format!(
                        "{ft} files run + {skipped} skipped, not the full {} of the pin (a filtered run?)",
                        pc.files
                    ));
                }
                Some(_) => {}
                None => {
                    status = status.max(Status::Open);
                    why.push(format!(
                        "no full-run file count for the pin {} in m0_exit.rs PIN_COUNTS",
                        c.pin.as_deref().unwrap_or("unknown")
                    ));
                }
            }
            // A skipped test is not run. One that fails without GC-stress (it is in the
            // expectation file) is no passing test. One that passes without it is named: the
            // skip list says why it is skipped (a timeout under GC-stress), and its decision row
            // records the run that passed it alone.
            let file = format!("test262.{}.jit.txt", c.o.host);
            let listed = committed(&c.o.root, &file).and_then(|p| expectation_set(&p));
            let skip =
                committed(&c.o.root, "test262-gc-skip.txt").and_then(|p| expectation_set(&p));
            let mut skip_text = String::new();
            let (mut new, mut stale) = (runner_new, runner_stale);
            let mut basis = "the runner's diff, not recomputed".to_string();
            match (listed, skip) {
                (Ok(listed), Ok(skip)) => {
                    let passing: Vec<&str> = skip
                        .iter()
                        .filter(|s| !listed.contains(*s))
                        .map(String::as_str)
                        .collect();
                    skip_text = format!(
                        "{skipped} skipped (expectations/test262-gc-skip.txt): {} fail without GC-stress too",
                        skip.len() - passing.len()
                    );
                    if !passing.is_empty() {
                        let _ = write!(
                            skip_text,
                            "; {} of them pass without it and are not run under it (a timeout skip, see the skip-list header): {}",
                            passing.len(),
                            passing.join(", ")
                        );
                    }
                    if skipped != skip.len() {
                        status = Status::Fail;
                        why.push(format!(
                            "the run skipped {skipped}, the skip list has {}",
                            skip.len()
                        ));
                    }
                    // The same tests, not only as many: another set of the same size hides a
                    // GC regression in the test skipped in place of a listed one.
                    match &skipped_paths {
                        None => {
                            status = Status::Fail;
                            why.push(
                                "the runner JSON has no skipped_paths (an older runner), so the skipped tests cannot be checked"
                                    .into(),
                            );
                        }
                        Some(paths) => {
                            if paths.len() != skipped {
                                status = Status::Fail;
                                why.push(format!(
                                    "skipped {skipped}, but skipped_paths names {}",
                                    paths.len()
                                ));
                            }
                            let extra: Vec<String> = paths.difference(&skip).cloned().collect();
                            let missing: Vec<String> = skip.difference(paths).cloned().collect();
                            if !extra.is_empty() {
                                status = Status::Fail;
                                why.push(format!(
                                    "skipped but not in the skip list: {}",
                                    some_paths(&extra)
                                ));
                            }
                            if !missing.is_empty() {
                                status = Status::Fail;
                                why.push(format!(
                                    "in the skip list but not skipped (run, or the entry names no test): {}",
                                    some_paths(&missing)
                                ));
                            }
                        }
                    }
                    // The diff against the committed file without the skip list, recomputed;
                    // the runner's diff counts too.
                    match recheck(&c.o.root, j, &file, &skip) {
                        Ok(d) => {
                            if !d.new.is_empty() {
                                why.push(format!("new failures: {}", some_paths(&d.new)));
                            }
                            new = new.max(d.new.len());
                            stale = stale.max(d.stale.len());
                            basis = format!("recomputed against expectations/{file}");
                        }
                        Err(e) => {
                            status = Status::Fail;
                            why.push(e);
                        }
                    }
                }
                (Err(e), _) | (_, Err(e)) => {
                    status = Status::Fail;
                    why.push(e);
                }
            }
            if infra > 0 || new > 0 {
                status = Status::Fail;
                why.push(format!("{infra} infrastructure errors, {new} new failures"));
            }
            if stale > 0 {
                why.push(format!("{stale} listed tests pass under GC-stress"));
            }
            cell(
                status,
                format!(
                    "{}: --gc-stress {gc}, files {fp} / {ft}, infra {infra}, new failures {new} ({basis}); {skip_text}{}",
                    p.display(),
                    if why.is_empty() {
                        String::new()
                    } else {
                        format!(". {}", why.join("; "))
                    }
                ),
            )
        }
    };
    let win = cell(
        Status::NotRun,
        "ci.yml runs the GC-stress step on linux-x64 only",
    );
    (host, win)
}

fn item7(c: &Ctx) -> (Cell, Cell) {
    let host = match &c.nesting {
        None => not_given("--nesting", NESTING_CMD),
        Some((p, log)) => {
            let mut status = Status::Pass;
            let mut parts = Vec::new();
            for t in NESTING_TESTS {
                let ok = format!("test {t} ... ok");
                let failed = format!("test {t} ... FAILED");
                let st = if log.lines().any(|l| l.trim_end() == ok) {
                    Status::Pass
                } else if log.lines().any(|l| l.trim_end() == failed) {
                    Status::Fail
                } else {
                    Status::NotRun
                };
                status = status.max(st);
                parts.push(format!("{t} {}", st.name()));
            }
            if log.contains("test result: FAILED") {
                status = Status::Fail;
                parts.push("a test binary FAILED".into());
            }
            if let Some(l) = cargo_failure(log) {
                status = Status::Fail;
                parts.push(format!("a cargo failure: {l:?}"));
            }
            cell(status, format!("{}: {}", p.display(), parts.join(", ")))
        }
    };
    let win = with_ci(c, |ci| {
        let s = ci.step("build + test", &["Run cargo test --workspace --locked"])?;
        Ok(cell(
            s.status(),
            format!(
                "step \"cargo test --workspace --locked\" {} (it runs {})",
                s.describe(),
                NESTING_TESTS.join(", ")
            ),
        ))
    });
    (host, win)
}

fn item8(c: &Ctx) -> (Cell, Cell) {
    let src = c.o.root.join("crates/ep9_value/src/lib.rs");
    let host = match read(&src) {
        Err(e) => cell(Status::Fail, e),
        Ok(text) => {
            let missing: Vec<&str> = STATIC_ASSERTS
                .iter()
                .filter(|a| !text.contains(&format!("assert!({a});")))
                .copied()
                .collect();
            if !missing.is_empty() {
                cell(
                    Status::Fail,
                    format!("crates/ep9_value/src/lib.rs lacks: {}", missing.join(", ")),
                )
            } else if !c.o.build_check {
                cell(
                    Status::NotRun,
                    "the asserts are in crates/ep9_value/src/lib.rs; --no-build-check skipped `cargo check`",
                )
            } else {
                let out = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
                    .current_dir(&c.o.root)
                    .args([
                        "check",
                        "--locked",
                        "--quiet",
                        "-p",
                        "ep9_value",
                        "--lib",
                        "--target",
                    ])
                    .arg(&c.o.host)
                    .output();
                match out {
                    Ok(o) if o.status.success() => cell(
                        Status::Pass,
                        format!(
                            "`const _` asserts in crates/ep9_value/src/lib.rs ({}); `cargo check -p ep9_value --target {}` evaluates them and passes",
                            STATIC_ASSERTS.join(", "),
                            c.o.host
                        ),
                    ),
                    Ok(o) => {
                        let err = String::from_utf8_lossy(&o.stderr);
                        let no_target = err.contains("can't find crate for `core`")
                            || err.contains("target may not be installed");
                        cell(
                            if no_target {
                                Status::NotRun
                            } else {
                                Status::Fail
                            },
                            format!(
                                "`cargo check -p ep9_value --target {}` failed: {}",
                                c.o.host,
                                err.lines()
                                    .find(|l| l.contains("error"))
                                    .unwrap_or("")
                                    .trim()
                            ),
                        )
                    }
                    Err(e) => cell(Status::NotRun, format!("cannot run cargo: {e}")),
                }
            }
        }
    };
    let win = with_ci(c, |ci| {
        let s = ci.step(
            "build + test",
            &["Run cargo build --workspace --all-targets --locked"],
        )?;
        Ok(cell(
            s.status(),
            format!(
                "step \"cargo build --workspace --all-targets --locked\" {} (it compiles the `const _` asserts for this target)",
                s.describe()
            ),
        ))
    });
    (host, win)
}

/// One row of the ADR index (docs/adr/README.md section 5).
#[derive(Debug, Clone)]
struct AdrRow {
    id: String,
    fix_by: String,
    status: String,
}

fn adr_index(text: &str) -> Result<Vec<AdrRow>, String> {
    let rows: Vec<AdrRow> = text
        .lines()
        .filter(|l| l.starts_with("| H"))
        .filter_map(|l| {
            let cols: Vec<&str> = l
                .split(" | ")
                .map(|c| c.trim_matches(|ch| ch == '|' || ch == ' '))
                .collect();
            (cols.len() >= 6 && cols[0][1..].bytes().all(|b| b.is_ascii_digit())).then(|| AdrRow {
                id: cols[0].to_string(),
                fix_by: cols[2].to_string(),
                status: cols[4].to_string(),
            })
        })
        .collect();
    if rows.len() < 37 {
        return Err(format!(
            "docs/adr/README.md: found {} index rows, expected H1-H37",
            rows.len()
        ));
    }
    Ok(rows)
}

fn merged(status: &str) -> bool {
    status.starts_with("Accepted") || status.starts_with("Frozen")
}

/// The README counts line, for the evidence.
fn counts_line(root: &Path) -> String {
    read(&root.join("docs/adr/README.md"))
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("Counts on"))
                .map(|l| l.split(". ").next().unwrap_or(l).to_string())
        })
        .unwrap_or_default()
}

fn item9(c: &Ctx) -> (Cell, Cell) {
    let due: Vec<&AdrRow> = c.adr.iter().filter(|r| r.fix_by.contains("M0")).collect();
    let open: Vec<String> = due
        .iter()
        .filter(|r| {
            if r.fix_by.contains("Draft M0") {
                !(r.status.starts_with("Draft") || merged(&r.status))
            } else {
                !merged(&r.status)
            }
        })
        .map(|r| r.id.clone())
        .collect();
    let h20 = c.adr.iter().find(|r| r.id == "H20");
    let h20_frozen = h20.is_some_and(|r| r.status.starts_with("Frozen"));
    let text = format!(
        "docs/adr/README.md index: {} of {} rows due at M0 merged (Accepted or Frozen; a Draft for \"Draft M0\" rows); not yet: {}; H20 (builtin ABI) {}. {}",
        due.len() - open.len(),
        due.len(),
        if open.is_empty() {
            "none".to_string()
        } else {
            open.join(" ")
        },
        if h20_frozen { "Frozen" } else { "not Frozen" },
        counts_line(&c.o.root)
    );
    let st = if open.is_empty() && h20_frozen {
        Status::Pass
    } else {
        Status::Open
    };
    (
        cell(st, text.clone()),
        cell(st, format!("repo-wide, as the host column: {text}")),
    )
}

fn item10(c: &Ctx) -> (Cell, Cell) {
    let mut open = Vec::new();
    for id in A0_PART1 {
        match c.adr.iter().find(|r| r.id == *id) {
            Some(r) if merged(&r.status) => {}
            Some(r) => open.push(format!(
                "{id} ({})",
                r.status.split(' ').next().unwrap_or("")
            )),
            None => open.push(format!("{id} (no row)")),
        }
    }
    let text = format!(
        "docs/adr/README.md index: {} of {} A0 part 1 ADRs Accepted or Frozen; not yet: {}",
        A0_PART1.len() - open.len(),
        A0_PART1.len(),
        if open.is_empty() {
            "none".to_string()
        } else {
            open.join(", ")
        }
    );
    let st = if open.is_empty() {
        Status::Pass
    } else {
        Status::Open
    };
    (
        cell(st, text.clone()),
        cell(st, format!("repo-wide, as the host column: {text}")),
    )
}

fn item11(c: &Ctx) -> (Cell, Cell) {
    let (st, t) = spike_status(c, "sp-unit", false);
    (cell(st, t.clone()), cell(st, t))
}

const CORPUS_CMD: &str = "cargo test -p ep9_frontend --release --locked --lib bytecodegen::corpus -- --ignored --nocapture > corpus-release.log 2>&1 (and the same without --release)";

fn item12(c: &Ctx) -> (Cell, Cell) {
    let want = read(&c.o.root.join("expectations/bytecode-corpus.digest"))
        .ok()
        .and_then(|t| {
            t.lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_string)
        })
        .unwrap_or_default();
    let host = if c.corpus.is_empty() {
        not_given("--corpus", CORPUS_CMD)
    } else {
        let mut status = Status::Pass;
        let mut parts = Vec::new();
        let mut profiles = BTreeSet::new();
        for (p, log) in &c.corpus {
            if let Some(l) = cargo_failure(log) {
                status = Status::Fail;
                parts.push(format!("{}: a cargo failure: {l:?}", p.display()));
            }
            let runs = corpus_runs(log);
            if runs.is_empty() {
                status = Status::Fail;
                parts.push(format!("{}: no \"corpus digest\" line", p.display()));
            }
            for r in runs {
                let ok = !r.failed && r.ok >= r.digests.len() && !r.digests.is_empty();
                if r.digests.is_empty() {
                    status = Status::Fail;
                    parts.push(format!(
                        "{} ({}): no \"corpus digest\" line",
                        p.display(),
                        r.profile
                    ));
                    continue;
                }
                if !ok || r.digests.iter().any(|(d, _)| *d != want) {
                    status = Status::Fail;
                } else {
                    profiles.insert(r.profile);
                }
                let digests: Vec<String> = r
                    .digests
                    .iter()
                    .map(|(d, units)| format!("{} over {units} units", short(d)))
                    .collect();
                parts.push(format!(
                    "{} ({}): {}, test {}",
                    p.display(),
                    r.profile,
                    digests.join(", "),
                    if ok { "ok" } else { "not ok" }
                ));
            }
        }
        if !(profiles.contains("release") && profiles.contains("debug")) {
            status = status.max(Status::Open);
            parts.push(format!(
                "a good release and a good debug run are needed (good: {})",
                if profiles.is_empty() {
                    "none".to_string()
                } else {
                    profiles.into_iter().collect::<Vec<_>>().join(", ")
                }
            ));
        }
        cell(
            status,
            format!(
                "{}; expectations/bytecode-corpus.digest = {}",
                parts.join("; "),
                short(&want)
            ),
        )
    };
    let win = with_ci(c, |ci| {
        let s = ci.step(
            "test262 parse + exec",
            &["bytecodegen corpus digest (release)"],
        )?;
        Ok(cell(
            s.status(),
            format!(
                "step \"bytecodegen corpus digest (release)\" {} (it asserts the digest equals expectations/bytecode-corpus.digest = {})",
                s.describe(),
                short(&want)
            ),
        ))
    });
    (host, win)
}

/// The first cargo or rustc failure in a log (items 7 and 12): a line that starts with
/// `error[E` or `error:` (a compile error, `error: could not compile`, `error: test failed`, a
/// crashed test binary, a usage or lockfile error) or a `thread '<name>' (<id>) panicked at`
/// line (older Rust prints no id). Such an
/// invocation can stop before its "Finished" line or its test result, so the log parsers would
/// count its lines in an earlier good run. A good corpus or nesting run prints none: the report
/// lines of the corpus test are indented counts and `e.g.` lines. A log that holds a failed
/// attempt and then a good run is rejected too (CORPUS_CMD writes one invocation per file).
fn cargo_failure(log: &str) -> Option<&str> {
    log.lines().map(str::trim).find(|t| {
        t.starts_with("error[E")
            || t.starts_with("error:")
            || (t.starts_with("thread '") && t.contains(" panicked at "))
    })
}

/// One cargo test run in a corpus log: from a "Finished `<profile>` profile" line to the next
/// one (a log can hold several runs, appended).
struct CorpusRun {
    profile: &'static str,
    /// (digest, units) of each `corpus digest <hex> over <n> units` line.
    digests: Vec<(String, String)>,
    /// "test result: ok. 1 passed" lines.
    ok: usize,
    /// A "test result: FAILED" line.
    failed: bool,
}

/// The runs of a corpus log. Lines before the first "Finished" line form a run of an unknown
/// profile, kept only if it has a digest or a test result.
fn corpus_runs(log: &str) -> Vec<CorpusRun> {
    let new = |profile| CorpusRun {
        profile,
        digests: Vec::new(),
        ok: 0,
        failed: false,
    };
    let mut runs = vec![new("unknown profile")];
    for l in log.lines() {
        let t = l.trim();
        if let Some(rest) = t.strip_prefix("Finished `") {
            runs.push(new(match rest.split('`').next() {
                Some("release") => "release",
                Some("test" | "dev") => "debug",
                _ => "unknown profile",
            }));
            continue;
        }
        let Some(r) = runs.last_mut() else { continue };
        if let Some((d, units)) = t
            .strip_prefix("corpus digest ")
            .and_then(|rest| rest.split_once(" over "))
        {
            r.digests.push((
                d.to_string(),
                units.split(' ').next().unwrap_or("").to_string(),
            ));
        } else if t.starts_with("test result: FAILED") {
            r.failed = true;
        } else if t.starts_with("test result: ok. 1 passed") {
            r.ok += 1;
        }
    }
    if runs[0].digests.is_empty() && runs[0].ok == 0 && !runs[0].failed {
        runs.remove(0);
    }
    runs
}

fn short(sha: &str) -> String {
    if sha.len() > 12 {
        format!("{}…{}", &sha[..8], &sha[sha.len() - 5..])
    } else {
        sha.to_string()
    }
}

// ---------------------------------------------------------------------------------------------
// Spike verdicts: markdown tables with a "Spike" and a "Verdict" column.

/// The newest `docs/measurements/*-m0-spike-review.md` (file names start with the date).
fn newest_spike_review(root: &Path) -> Option<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(root.join("docs/measurements"))
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("-m0-spike-review.md"))
        })
        .collect();
    v.sort();
    v.pop()
}

/// (spike name in lower case, verdict text) for each row of each table that has a "Spike" and a
/// "Verdict" column. A cell "Spike B, SP-Unit" names two spikes.
fn verdict_table(md: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cols: Option<(usize, usize)> = None;
    for line in md.lines() {
        let l = line.trim();
        if !l.starts_with('|') {
            cols = None;
            continue;
        }
        let cells: Vec<&str> = l.trim_matches('|').split('|').map(str::trim).collect();
        if cols.is_none() {
            let s = cells.iter().position(|c| *c == "Spike");
            let v = cells.iter().position(|c| *c == "Verdict");
            if let (Some(s), Some(v)) = (s, v) {
                cols = Some((s, v));
            }
            continue;
        }
        let (s, v) = cols.unwrap_or((0, 0));
        if cells
            .iter()
            .all(|c| c.chars().all(|ch| ch == '-' || ch == ':'))
        {
            continue;
        }
        let (Some(name), Some(verdict)) = (cells.get(s), cells.get(v)) else {
            continue;
        };
        for n in name.split(',') {
            let n = n.trim().trim_matches('*').to_ascii_lowercase();
            if !n.is_empty() {
                out.push((n, verdict.trim_matches('*').to_string()));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// CI run: `gh run view <id> --json jobs,headSha,databaseId,url`.

#[derive(Debug)]
struct Ci {
    id: Option<String>,
    head: Option<String>,
    url: Option<String>,
    jobs: Vec<Job>,
}

#[derive(Debug)]
struct Job {
    name: String,
    steps: Vec<Step>,
}

#[derive(Debug)]
struct Step {
    name: String,
    status: String,
    conclusion: String,
    started: Option<i64>,
    completed: Option<i64>,
}

impl Step {
    fn status(&self) -> Status {
        match (self.status.as_str(), self.conclusion.as_str()) {
            ("completed", "success") => Status::Pass,
            ("completed", "failure" | "timed_out" | "startup_failure") => Status::Fail,
            // skipped, cancelled, neutral; or queued, in_progress
            _ => Status::NotRun,
        }
    }

    fn seconds(&self) -> Option<i64> {
        Some(self.completed? - self.started?)
    }

    fn describe(&self) -> String {
        if self.status == "completed" {
            self.conclusion.clone()
        } else {
            format!("{} (not finished)", self.status)
        }
    }
}

/// A JSON string or number as text.
fn text(v: Option<&Json>) -> Option<String> {
    match v? {
        Json::Str(s) => Some(s.clone()),
        Json::Num(n) => Some(format!("{n:.0}")),
        _ => None,
    }
}

impl Ci {
    fn from_json(j: &Json) -> Result<Ci, String> {
        let jobs = j
            .get("jobs")
            .and_then(Json::arr)
            .ok_or("the CI run JSON has no \"jobs\" array (gh run view <id> --json jobs,...)")?;
        let step = |s: &Json| Step {
            name: text(s.get("name")).unwrap_or_default(),
            status: text(s.get("status")).unwrap_or_default(),
            conclusion: text(s.get("conclusion")).unwrap_or_default(),
            started: text(s.get("startedAt")).and_then(|t| iso_seconds(&t)),
            completed: text(s.get("completedAt")).and_then(|t| iso_seconds(&t)),
        };
        Ok(Ci {
            id: text(j.get("databaseId")),
            head: text(j.get("headSha")),
            url: text(j.get("url")),
            jobs: jobs
                .iter()
                .map(|job| Job {
                    name: text(job.get("name")).unwrap_or_default(),
                    steps: job
                        .get("steps")
                        .and_then(Json::arr)
                        .unwrap_or(&[])
                        .iter()
                        .map(step)
                        .collect(),
                })
                .collect(),
        })
    }

    /// The step named one of `names` in the job `<job> (windows-x64)`.
    fn step(&self, job: &str, names: &[&str]) -> Result<&Step, String> {
        let full = format!("{job} ({CI_MATRIX})");
        let j = self
            .jobs
            .iter()
            .find(|j| j.name == full)
            .ok_or_else(|| format!("the run has no job \"{full}\""))?;
        j.steps
            .iter()
            .find(|s| names.contains(&s.name.as_str()))
            .ok_or_else(|| format!("job \"{full}\" has no step {:?}", names[0]))
    }
}

/// The Windows cell from the CI run: NOT-RUN without a run; a missing job or step is NOT-RUN;
/// an input file that cannot be read is NOT-RUN with the error.
fn with_ci(c: &Ctx, f: impl FnOnce(&Ci) -> Result<Cell, String>) -> Cell {
    match &c.ci {
        Err(e) => cell(Status::NotRun, e.clone()),
        Ok(ci) => f(ci).unwrap_or_else(|e| cell(Status::NotRun, e)),
    }
}

/// `gh run view <id> --json jobs,headSha,databaseId,url`. Err (the Windows column reads
/// NOT-RUN) when gh is missing or the run does not exist.
fn gh_run(id: &str) -> Result<Ci, String> {
    let out = Command::new("gh")
        .args(["run", "view", id, "--json", "jobs,headSha,databaseId,url"])
        .output()
        .map_err(|e| format!("cannot run gh: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "gh run view {id} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let j = parse_json(&String::from_utf8_lossy(&out.stdout))
        .map_err(|e| format!("gh run view {id}: {e}"))?;
    Ci::from_json(&j)
}

/// Seconds since 1970 of an RFC 3339 UTC time as GitHub writes it ("2026-10-22T01:02:03Z",
/// fractions allowed). None for anything else (GitHub writes "0001-01-01T00:00:00Z" for a step
/// that did not start; that parses, and the caller sees a nonsense duration only if both ends
/// are set).
fn iso_seconds(t: &str) -> Option<i64> {
    let b = t.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    if !t.ends_with('Z') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| t.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    // Days from civil (H. Hinnant), proleptic Gregorian.
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3_600 + mi * 60 + s)
}

fn read(p: &Path) -> Result<String, String> {
    std::fs::read_to_string(p).map_err(|e| format!("cannot read {}: {e}", p.display()))
}

/// The trimmed stdout of `git -C <dir> <args>`, or None.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    (out.status.success() && !s.trim().is_empty()).then(|| s.trim().to_string())
}

// ---------------------------------------------------------------------------------------------
// A small JSON reader (RFC 8259): objects, arrays, strings, numbers, true, false, null.

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    /// Members in source order; a repeated key keeps its first value for `get`.
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn path(&self, keys: &[&str]) -> Option<&Json> {
        keys.iter().try_fold(self, |j, k| j.get(k))
    }

    fn str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    fn num(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    fn bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
}

/// Nesting deeper than this is an error, not a stack overflow.
const MAX_JSON_DEPTH: usize = 256;

fn parse_json(text: &str) -> Result<Json, String> {
    let mut p = JsonParser {
        s: text,
        b: text.as_bytes(),
        i: 0,
    };
    let v = p.value(0)?;
    p.ws();
    if p.i != p.b.len() {
        return Err(p.err("trailing characters"));
    }
    Ok(v)
}

struct JsonParser<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
}

impl JsonParser<'_> {
    fn err(&self, what: &str) -> String {
        let line = 1 + self.b[..self.i.min(self.b.len())]
            .iter()
            .filter(|&&c| c == b'\n')
            .count();
        format!("JSON: {what} at byte {} (line {line})", self.i)
    }

    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(self.err("bad literal"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_JSON_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        self.ws();
        match self.b.get(self.i) {
            None => Err(self.err("unexpected end")),
            Some(b'{') => {
                self.i += 1;
                let mut m = Vec::new();
                if self.eat(b'}') {
                    return Ok(Json::Obj(m));
                }
                loop {
                    self.ws();
                    if self.b.get(self.i) != Some(&b'"') {
                        return Err(self.err("expected a key"));
                    }
                    let k = self.string()?;
                    if !self.eat(b':') {
                        return Err(self.err("expected ':'"));
                    }
                    let v = self.value(depth + 1)?;
                    m.push((k, v));
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b'}') {
                        return Ok(Json::Obj(m));
                    }
                    return Err(self.err("expected ',' or '}'"));
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                if self.eat(b']') {
                    return Ok(Json::Arr(a));
                }
                loop {
                    a.push(self.value(depth + 1)?);
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b']') {
                        return Ok(Json::Arr(a));
                    }
                    return Err(self.err("expected ',' or ']'"));
                }
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'n') => self.lit("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("unexpected character")),
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        let digits = |p: &mut Self| {
            let s = p.i;
            while p.i < p.b.len() && p.b[p.i].is_ascii_digit() {
                p.i += 1;
            }
            p.i > s
        };
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        let int_start = self.i;
        if !digits(self) || (self.b[int_start] == b'0' && self.i - int_start > 1) {
            return Err(self.err("bad number"));
        }
        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            if !digits(self) {
                return Err(self.err("bad number"));
            }
        }
        if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !digits(self) {
                return Err(self.err("bad number"));
            }
        }
        self.s[start..self.i]
            .parse::<f64>()
            .map(Json::Num)
            .map_err(|_| self.err("bad number"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self
            .s
            .get(self.i..self.i + 4)
            .filter(|h| h.bytes().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| self.err("bad \\u escape"))?;
        self.i += 4;
        u32::from_str_radix(h, 16).map_err(|_| self.err("bad \\u escape"))
    }

    /// A string; `self.i` is at the opening quote. A lone surrogate becomes U+FFFD.
    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let run = self.i;
            while self.i < self.b.len() && !matches!(self.b[self.i], b'"' | b'\\' | 0..=0x1f) {
                self.i += 1;
            }
            out.push_str(&self.s[run..self.i]);
            match self.b.get(self.i) {
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let e = *self
                        .b
                        .get(self.i)
                        .ok_or_else(|| self.err("unexpected end"))?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let c = if (0xd800..0xdc00).contains(&hi)
                                && self.b[self.i..].starts_with(b"\\u")
                            {
                                let save = self.i;
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xdc00..0xe000).contains(&lo) {
                                    char::from_u32(0x10000 + ((hi - 0xd800) << 10) + (lo - 0xdc00))
                                } else {
                                    self.i = save;
                                    None
                                }
                            } else {
                                char::from_u32(hi)
                            };
                            out.push(c.unwrap_or('\u{fffd}'));
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
                Some(_) => return Err(self.err("control character in a string")),
                None => return Err(self.err("unterminated string")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_reader() {
        let j = parse_json(
            r#" {"a": [1, -2.5e2, true, false, null], "s": "q\"\\\/\b\f\n\r\t\u00e9\ud83d\ude00\ud800x", "o": {}} "#,
        )
        .unwrap();
        assert_eq!(
            j.get("a").and_then(Json::arr).unwrap(),
            &[
                Json::Num(1.0),
                Json::Num(-250.0),
                Json::Bool(true),
                Json::Bool(false),
                Json::Null
            ]
        );
        assert_eq!(
            j.get("s").and_then(Json::str),
            Some("q\"\\/\u{8}\u{c}\n\r\té😀\u{fffd}x")
        );
        assert_eq!(j.get("o"), Some(&Json::Obj(vec![])));
        assert_eq!(j.path(&["o", "x"]), None);
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\" 1}",
            "01",
            "1.",
            "-",
            "\"\u{1}\"",
            "\"\\x\"",
            "[1] 2",
            "tru",
            "\"\\u12\"",
            "{1: 2}",
        ] {
            assert!(parse_json(bad).is_err(), "{bad:?}");
        }
        let deep = "[".repeat(MAX_JSON_DEPTH + 2);
        assert!(parse_json(&deep).unwrap_err().contains("too deep"));
    }

    #[test]
    fn iso_times() {
        assert_eq!(iso_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_seconds("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(iso_seconds("2026-10-22T01:02:03Z"), Some(1_792_630_923));
        assert_eq!(iso_seconds("2026-10-22T01:02:03.5Z"), Some(1_792_630_923));
        assert_eq!(iso_seconds("2026-10-22 01:02:03"), None);
        assert_eq!(iso_seconds("2026-13-22T01:02:03Z"), None);
    }

    #[test]
    fn verdict_tables() {
        let md = "x\n\n| Spike | Gate | Verdict | ADRs |\n|---|---|---|---|\n| SP-GC | g | GO | H9 |\n\
                  | SP-Dispatch | g | GO with conditions (§3) | H15 |\n| Spike B, SP-Unit | — | not reviewed | — |\n\n\
                  | Other | Verdict |\n|---|---|\n| x | y |\n";
        let v = verdict_table(md);
        assert_eq!(
            v,
            [
                ("sp-gc", "GO"),
                ("sp-dispatch", "GO with conditions (§3)"),
                ("spike b", "not reviewed"),
                ("sp-unit", "not reviewed"),
            ]
            .map(|(a, b)| (a.to_string(), b.to_string()))
        );
    }

    #[test]
    fn the_real_adr_index_has_37_rows() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let rows = adr_index(&read(&root.join("docs/adr/README.md")).unwrap()).unwrap();
        assert_eq!(rows.len(), 37);
        assert_eq!(rows[19].id, "H20");
        assert!(rows.iter().any(|r| r.fix_by.contains("Draft M0")));
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    const HOST: &str = "aarch64-apple-darwin";

    /// The skip list of [`fake_repo`].
    const SKIP: [&str; 2] = ["test/skip/one.js", "test/skip/two.js"];

    /// An ep9-test262 exec JSON: `gc` the --gc-stress value, `skipped` the paths the skip list
    /// skipped, `fails` the paths with status "fail", `used` the expectation file it names. The
    /// runner's own diff is empty; the timeout and feedback threshold are those of [`GC_CMD`].
    fn exec_json(gc: u32, wall: f64, skipped: &[&str], fails: &[&str], used: &str) -> String {
        let results: Vec<String> = fails
            .iter()
            .map(|p| format!(r#"{{"path":"{p}","status":"fail","cause":"x","detail":"d"}}"#))
            .collect();
        let files = PIN_COUNTS[0].files - skipped.len() as u64;
        let paths: Vec<String> = skipped.iter().map(|p| json_str(p)).collect();
        format!(
            r#"{{"command":"exec","mode":"jit","target":"{HOST}","workers":18,"timeout_ms":60000,"gc_stress":{gc},"feedback_threshold":0,"skipped":{},"skipped_paths":[{}],"wall_seconds":{wall},
            "files":{{"passed":{},"total":{files}}},"runs":{{"passed":50000,"total":102994}},
            "by_dir":{{"language":{{"files":{{"passed":4,"total":6}},"runs":{{"passed":1,"total":2}}}}}},
            "causes":[],"results":[{}],
            "expectations":{{"file":{},"new_failures":[],"now_passing":[]}}}}"#,
            skipped.len(),
            paths.join(","),
            files - fails.len() as u64,
            results.join(","),
            json_str(used)
        )
    }

    /// `s` as a JSON string (a Windows temp path has backslashes).
    fn json_str(s: &str) -> String {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }

    /// A fake repo with every input, from the real ADR README and spike review.
    fn fake_repo(name: &str) -> PathBuf {
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let root = std::env::temp_dir().join(format!("ep9-m0-exit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        write(&root, "third_party/test262.commit", "d53ebc8b0000\n");
        write(
            &root,
            "docs/adr/README.md",
            &read(&real.join("docs/adr/README.md")).unwrap(),
        );
        write(
            &root,
            "docs/measurements/2026-10-21-m0-spike-review.md",
            &read(&real.join("docs/measurements/2026-10-21-m0-spike-review.md")).unwrap(),
        );
        write(
            &root,
            "crates/ep9_value/src/lib.rs",
            &read(&real.join("crates/ep9_value/src/lib.rs")).unwrap(),
        );
        write(
            &root,
            "expectations/bytecode-corpus.digest",
            "# c\nabc123\n",
        );
        let fails =
            "# h\ntest/language/a.js\ntest/language/b.js\ntest/built-ins/c.js\ntest/skip/one.js\n";
        write(
            &root,
            &format!("expectations/test262.{HOST}.jit.txt"),
            fails,
        );
        write(
            &root,
            &format!("expectations/test262.{CI_TRIPLE}.jit.txt"),
            fails,
        );
        write(
            &root,
            "expectations/test262-gc-skip.txt",
            "# s\ntest/skip/one.js\ntest/skip/two.js\n",
        );
        write(
            &root,
            &format!("expectations/test262-parse.{HOST}.txt"),
            "test/p.js\n",
        );
        write(
            &root,
            &format!("expectations/test262-parse.{CI_TRIPLE}.txt"),
            "# w\ntest/p.js\n",
        );
        let used = root.join(format!("expectations/test262.{HOST}.jit.txt"));
        let used = used.to_str().unwrap();
        // The default run fails every listed test; the GC run skips the 2 skip-list tests.
        let all = [
            "test/language/a.js",
            "test/language/b.js",
            "test/built-ins/c.js",
            "test/skip/one.js",
        ];
        write(&root, "in/exec.json", &exec_json(0, 5.4, &[], &all, used));
        write(
            &root,
            "in/gc.json",
            &exec_json(1, 166.0, &SKIP, &all[..3], used),
        );
        write(&root, "in/slow.json", &exec_json(0, 700.0, &[], &all, used));
        write(
            &root,
            "in/parse.json",
            &format!(
                r#"{{"command":"parse","target":"{HOST}","negatives":{{"all":{{"passed":4653,"total":4653}}}},
                "positives":{{"all":{{"passed":47357,"total":47364}}}}}}"#
            ),
        );
        write(
            &root,
            "in/cross.json",
            r#"{"schema":"ep9-cross/v1","commit":{"ep9":"fa451f6"},"provisional":false,"host":{"gate_host":true},
              "M":{"ratios":[
                {"workload":"richards","comparator":"qjs","metric":"net","void":false,"net":{"median":1.39,"interval":[1.37,1.43]}},
                {"workload":"deltablue","comparator":"qjs","metric":"net","void":false,"net":{"median":1.56,"interval":[1.51,1.58]}},
                {"workload":"richards","comparator":"node-jitless","metric":"net","void":false,"net":{"median":0.1,"interval":[0.1,0.1]}}]}}"#,
        );
        write(
            &root,
            "in/nesting.log",
            "test depth_tests::every_form_is_gated ... ok\ntest result: ok. 1 passed\n\
             test caps_hold_end_to_end ... ok\ntest uncaught_error_reports_file_line_utf16_column ... ok\n",
        );
        write(
            &root,
            "in/corpus.log",
            &corpus_log("release", "abc123", true),
        );
        write(
            &root,
            "in/corpus-debug.log",
            &corpus_log("test", "abc123", true),
        );
        root
    }

    /// The output of one corpus test run in the `profile` cargo profile.
    fn corpus_log(profile: &str, digest: &str, ok: bool) -> String {
        format!(
            "    Finished `{profile}` profile [optimized] target(s) in 1s\n     Running unittests src/lib.rs\n\
             running 1 test\ncorpus digest {digest} over 77183 units (compare across hosts and targets)\n{}",
            if ok {
                "test bytecodegen::corpus::corpus_digest ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored\n"
            } else {
                "test bytecodegen::corpus::corpus_digest ... FAILED\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored\n"
            }
        )
    }

    fn step(name: &str, conclusion: &str, from: &str, to: &str) -> String {
        format!(
            r#"{{"name":"{name}","status":"completed","conclusion":"{conclusion}","number":1,"startedAt":"2026-10-21T23:{from}Z","completedAt":"2026-10-21T23:{to}Z"}}"#
        )
    }

    /// A run in the shape of `gh run view <id> --json jobs,headSha,databaseId,url`.
    fn ci_json(exec_conclusion: &str) -> String {
        ci_json_with(|name| {
            Some(if name == "test262 exec" {
                exec_conclusion
            } else {
                "success"
            })
        })
    }

    /// The same run, with `conclusion(step name)` for each step; None leaves the step out.
    fn ci_json_with<'a>(conclusion: impl Fn(&str) -> Option<&'a str>) -> String {
        let job = |name: &str, steps: &[(&str, &str, &str)]| {
            let steps: Vec<String> = steps
                .iter()
                .filter_map(|&(n, from, to)| conclusion(n).map(|c| step(n, c, from, to)))
                .collect();
            format!(
                r#"{{"name":"{name} (windows-x64)","status":"completed","conclusion":"success","databaseId":7,"steps":[{}]}}"#,
                steps.join(",")
            )
        };
        format!(
            r#"{{"databaseId":39636336281,"headSha":"44c5dcb","url":"https://x","jobs":[{},{},{},{}]}}"#,
            job(
                "build + test",
                &[
                    (
                        "Run cargo build --workspace --all-targets --locked",
                        "02:00",
                        "12:00"
                    ),
                    ("Run cargo test --workspace --locked", "12:00", "30:00"),
                    ("spike-native tests (ep9_vm)", "30:00", "33:00"),
                ]
            ),
            job(
                "test262 parse + exec",
                &[
                    ("test262 parse", "04:00", "14:00"),
                    ("test262 exec", "14:00", "15:12"),
                    ("bytecodegen corpus digest (release)", "16:30", "22:00"),
                ]
            ),
            job("disasm checks", &[("ep9_heap barrier", "15:00", "16:00")]),
            job(
                "spike gates",
                &[
                    ("SP-Dispatch gate", "05:00", "30:00"),
                    ("SP-GC gate", "30:00", "38:00"),
                    ("SP-JIT-mem gate", "38:00", "44:00"),
                    ("Spike B structural gate", "44:00", "50:00"),
                ]
            ),
        )
    }

    fn opts(root: &Path) -> Options {
        let i = |f: &str| Some(root.join("in").join(f));
        Options {
            root: root.to_path_buf(),
            host: HOST.into(),
            out: root.join("out.md"),
            exec: i("exec.json"),
            parse: i("parse.json"),
            exec_gc: i("gc.json"),
            cross: i("cross.json"),
            nesting: i("nesting.log"),
            corpus: vec![root.join("in/corpus.log"), root.join("in/corpus-debug.log")],
            verdicts: vec![],
            ci_run: Some("39636336281".into()),
            ci_run_json: i("ci.json"),
            build_check: false,
        }
    }

    fn statuses(r: &Report) -> Vec<(&'static str, &'static str)> {
        r.cells
            .iter()
            .map(|(h, w)| (h.status.name(), w.status.name()))
            .collect()
    }

    #[test]
    fn a_full_report_from_fake_inputs() {
        let root = fake_repo("full");
        write(&root, "in/ci.json", &ci_json("success"));
        let r = run(&opts(&root)).unwrap();
        let s = statuses(&r);
        assert_eq!(s[0], ("PASS", "PASS"), "{}", r.markdown);
        assert_eq!(s[1], ("PASS", "PASS"), "{}", r.markdown);
        assert_eq!(s[2], ("PASS", "PASS"), "{}", r.markdown);
        assert_eq!(s[3], ("PASS", "NOT-RUN"), "{}", r.markdown);
        // SP-Dispatch has open conditions and Spike B is not reviewed.
        assert_eq!(s[4], ("OPEN", "OPEN"), "{}", r.markdown);
        assert_eq!(s[5], ("PASS", "NOT-RUN"), "{}", r.markdown);
        assert_eq!(s[6], ("PASS", "PASS"), "{}", r.markdown);
        assert_eq!(s[7], ("NOT-RUN", "PASS"), "{}", r.markdown);
        assert_eq!(s[8], ("OPEN", "OPEN"));
        assert_eq!(s[9], ("OPEN", "OPEN"));
        assert_eq!(s[10], ("OPEN", "OPEN"));
        assert_eq!(s[11], ("PASS", "PASS"), "{}", r.markdown);
        assert!(!r.all_pass());
        // 4 of 6 language files pass on Windows: 2 of the listed failures are language/.
        assert!(
            r.cells[2].1.evidence.contains("4 / 6"),
            "{}",
            r.cells[2].1.evidence
        );
        // The skipped test that passes without GC-stress is named.
        assert!(
            r.cells[5].0.evidence.contains("test/skip/two.js"),
            "{}",
            r.cells[5].0.evidence
        );
        assert!(
            r.markdown
                .contains("CI run 39636336281 on commit `44c5dcb`")
        );
        assert_eq!(r.table.lines().count(), 14);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failures_and_missing_inputs() {
        let root = fake_repo("fail");
        write(&root, "in/ci.json", &ci_json("failure"));
        let mut o = opts(&root);
        o.exec = Some(root.join("in/slow.json"));
        o.parse = None;
        o.nesting = None;
        write(
            &root,
            "in/corpus.log",
            "corpus digest def456 over 1 units\ntest result: ok. 1 passed\n",
        );
        let r = run(&o).unwrap();
        let s = statuses(&r);
        assert_eq!(s[0], ("FAIL", "FAIL"), "{}", r.markdown);
        assert_eq!(s[1].0, "NOT-RUN");
        assert!(r.cells[1].0.evidence.contains("parse --json"));
        assert_eq!(s[6].0, "NOT-RUN");
        assert_eq!(s[11].0, "FAIL");
        // A wrong run id, a wrong host and a JSON of the wrong kind are usage errors.
        o.ci_run = Some("1".into());
        assert!(run(&o).unwrap_err().contains("holds run 39636336281"));
        o.ci_run = None;
        o.host = "x86_64-unknown-linux-gnu".into();
        assert!(run(&o).unwrap_err().contains("--host"));
        o.host = HOST.into();
        o.exec = Some(root.join("in/parse.json"));
        assert!(
            run(&o)
                .unwrap_err()
                .contains("not an ep9-test262 exec JSON")
        );
        // No run: the Windows column is NOT-RUN.
        o.exec = Some(root.join("in/exec.json"));
        o.ci_run_json = None;
        let r = run(&o).unwrap();
        assert!(
            r.cells
                .iter()
                .all(|(_, w)| w.status != Status::Pass || w.evidence.starts_with("repo-wide"))
        );
        assert!(r.cells[0].1.evidence.contains("no CI run given"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// I3: the runner leaves its diff empty when it cannot read its expectation file, and it
    /// can be pointed at any file, so item 1 recomputes the diff from the results against the
    /// committed file and needs the run to name that file.
    #[test]
    fn item1_recomputes_the_diff() {
        let root = fake_repo("item1-diff");
        write(&root, "in/ci.json", &ci_json("success"));
        let all = [
            "test/language/a.js",
            "test/language/b.js",
            "test/built-ins/c.js",
            "test/skip/one.js",
        ];
        let committed = root.join(format!("expectations/test262.{HOST}.jit.txt"));
        let committed = committed.to_str().unwrap();
        let missing = format!("/nonexistent/dir/test262.{HOST}.jit.txt");
        let item1 = |fails: &[&str], used: &str| {
            write(&root, "in/exec.json", &exec_json(0, 5.4, &[], fails, used));
            let r = run(&opts(&root)).unwrap();
            r.cells[0].0.clone()
        };
        // A failed read: the runner names a file that does not exist and diffs nothing.
        let c = item1(&all, &missing);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("not recomputed"), "{}", c.evidence);
        assert!(c.evidence.contains("/nonexistent/dir"), "{}", c.evidence);
        // A file outside this tree with the right name.
        write(
            &root,
            &format!("other/test262.{HOST}.jit.txt"),
            "test/x.js\n",
        );
        let other = root.join(format!("other/test262.{HOST}.jit.txt"));
        let c = item1(&all, other.to_str().unwrap());
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        // A new failure that the runner's (empty) diff does not report.
        let mut more = all.to_vec();
        more.push("test/new/x.js");
        let c = item1(&more, committed);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("new failures 1"), "{}", c.evidence);
        // A listed test that passes: stale, OPEN.
        let c = item1(&all[..3], committed);
        assert_eq!(c.status, Status::Open, "{}", c.evidence);
        assert!(c.evidence.contains("stale lines 1"), "{}", c.evidence);
        // The committed file and the same failing set: PASS.
        let c = item1(&all, committed);
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// I2: item 6 needs the full pinned corpus (run plus skipped files), the committed
    /// expectation file, and the diff recomputed from the results without the skip list.
    #[test]
    fn gc_run_needs_full_coverage() {
        let root = fake_repo("gc-cover");
        write(&root, "in/ci.json", &ci_json("success"));
        let listed = [
            "test/language/a.js",
            "test/language/b.js",
            "test/built-ins/c.js",
        ];
        let committed = root.join(format!("expectations/test262.{HOST}.jit.txt"));
        let committed = committed.to_str().unwrap();
        let item6 = |json: String| {
            write(&root, "in/gc.json", &json);
            let r = run(&opts(&root)).unwrap();
            r.cells[5].0.clone()
        };
        // A filtered GC run: 1 file run, the 2 of the skip list skipped, an empty diff.
        let full = exec_json(1, 166.0, &SKIP, &listed, committed);
        let files = format!(r#""files":{{"passed":{},"total":{}}}"#, 53_614 - 3, 53_614);
        assert!(full.contains(&files), "{full}");
        let c = item6(full.replace(&files, r#""files":{"passed":1,"total":1}"#));
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("not the full"), "{}", c.evidence);
        // Compared with a file outside this tree.
        let c = item6(exec_json(1, 166.0, &SKIP, &listed, "/elsewhere/other.txt"));
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("not recomputed"), "{}", c.evidence);
        assert!(
            c.evidence.contains("/elsewhere/other.txt"),
            "{}",
            c.evidence
        );
        // A new failure under GC-stress that the runner's (empty) diff does not report.
        let mut more = listed.to_vec();
        more.push("test/new/x.js");
        let c = item6(exec_json(1, 166.0, &SKIP, &more, committed));
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("new failures 1"), "{}", c.evidence);
        // The full corpus against the committed file: PASS.
        let c = item6(exec_json(1, 166.0, &SKIP, &listed, committed));
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// M2-P2-RF2-I8: item 6 needs the run to skip exactly the committed skip list (by path, not
    /// by count), with the timeout and the feedback threshold of [`GC_CMD`].
    #[test]
    fn gc_run_skips_exactly_the_skip_list() {
        let root = fake_repo("gc-skip-set");
        write(&root, "in/ci.json", &ci_json("success"));
        let listed = [
            "test/language/a.js",
            "test/language/b.js",
            "test/built-ins/c.js",
        ];
        let committed = root.join(format!("expectations/test262.{HOST}.jit.txt"));
        let committed = committed.to_str().unwrap();
        let item6 = |json: String| {
            write(&root, "in/gc.json", &json);
            let r = run(&opts(&root)).unwrap();
            r.cells[5].0.clone()
        };
        let honest = exec_json(1, 166.0, &SKIP, &listed, committed);
        let c = item6(honest.clone());
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        // The same count, another set: two.js (passing, the timeout skip) ran and passed, and a
        // passing test z.js was skipped, so a GC regression in z.js is hidden.
        let c = item6(exec_json(
            1,
            166.0,
            &["test/skip/one.js", "test/skip/z.js"],
            &listed,
            committed,
        ));
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("test/skip/z.js"), "{}", c.evidence);
        assert!(c.evidence.contains("test/skip/two.js"), "{}", c.evidence);
        // A subset and a superset.
        let c = item6(exec_json(1, 166.0, &SKIP[..1], &listed, committed));
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        let c = item6(exec_json(
            1,
            166.0,
            &[SKIP[0], SKIP[1], "test/skip/z.js"],
            &listed,
            committed,
        ));
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        // An older runner: no paths, only the count.
        let paths = r#""skipped_paths":["test/skip/one.js","test/skip/two.js"],"#;
        assert!(honest.contains(paths), "{honest}");
        let c = item6(honest.replace(paths, ""));
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("skipped_paths"), "{}", c.evidence);
        // A count that does not match the paths.
        let c = item6(honest.replace(paths, r#""skipped_paths":["test/skip/one.js"],"#));
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        // Not the timeout or the feedback threshold of GC_CMD (a longer timeout lets a timeout
        // skip pass; lazy feedback is another run). A shorter timeout is stricter.
        for (from, to) in [
            (r#""timeout_ms":60000,"#, r#""timeout_ms":600000,"#),
            (r#""timeout_ms":60000,"#, ""),
            (r#""feedback_threshold":0,"#, r#""feedback_threshold":8,"#),
            (r#""feedback_threshold":0,"#, ""),
        ] {
            assert!(honest.contains(from), "{honest}");
            let c = item6(honest.replace(from, to));
            assert_eq!(c.status, Status::Fail, "{to}: {}", c.evidence);
        }
        let c = item6(honest.replace(r#""timeout_ms":60000,"#, r#""timeout_ms":15000,"#));
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// I4: the Windows cell of item 5 needs the spike-native tests (D21, the only Windows run
    /// of Spike B's native code) and the Spike B structural gate, not only the other gates.
    #[test]
    fn windows_spike_b_needs_its_steps() {
        let root = fake_repo("win-spike-b");
        write(
            &root,
            "in/verdicts.md",
            "| Spike | Verdict |\n|---|---|\n| SP-GC, SP-VA, SP-Value, SP-Dispatch, SP-JIT-mem, Spike B | GO |\n\
             | SP-tsrs-win | recorded |\n",
        );
        let mut o = opts(&root);
        o.verdicts = vec![root.join("in/verdicts.md")];
        let item5 = |ci: String| {
            write(&root, "in/ci.json", &ci);
            let r = run(&o).unwrap();
            assert_eq!(
                r.cells[4].0.status,
                Status::Pass,
                "{}",
                r.cells[4].0.evidence
            );
            r.cells[4].1.clone()
        };
        for step in ["spike-native tests (ep9_vm)", "Spike B structural gate"] {
            let c = item5(ci_json_with(|n| {
                Some(if n == step { "failure" } else { "success" })
            }));
            assert_eq!(c.status, Status::Fail, "{step}: {}", c.evidence);
            assert!(c.evidence.contains(step), "{}", c.evidence);
            let c = item5(ci_json_with(|n| (n != step).then_some("success")));
            assert_eq!(c.status, Status::NotRun, "{step}: {}", c.evidence);
        }
        let c = item5(ci_json("success"));
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// I5: a gross fallback row (net time <= 0 in a round, so no net ratio) passes or fails
    /// nothing, as tools/bench/cross/run.mjs label() says; item 4 reads net ratios only.
    #[test]
    fn canary_ignores_gross_fallback() {
        let root = fake_repo("canary");
        let row = |w: &str, valid: bool| {
            if valid {
                format!(
                    r#"{{"workload":"{w}","comparator":"qjs","metric":"net","void":false,"net_invalid":null,
                    "net":{{"median":0.8,"interval":[0.7,0.9],"void":false}},"gross":{{"median":0.5,"interval":[0.4,0.6],"void":false}}}}"#
                )
            } else {
                format!(
                    r#"{{"workload":"{w}","comparator":"qjs","metric":"gross","void":false,"net":null,
                    "net_invalid":"net time <= 0 in round(s) 3","gross":{{"median":0.8,"interval":[0.7,0.9],"void":false}}}}"#
                )
            }
        };
        let canary = |valid: bool| {
            let j = format!(
                r#"{{"schema":"ep9-cross/v1","commit":{{"ep9":"fa451f6"}},"provisional":false,"host":{{"gate_host":true}},
                "M":{{"ratios":[{},{}]}}}}"#,
                row("richards", valid),
                row("deltablue", valid)
            );
            cross_canary(Path::new("run.json"), &parse_json(&j).unwrap(), &root)
        };
        let c = canary(false);
        assert_eq!(c.status, Status::Open, "{}", c.evidence);
        assert!(c.evidence.contains("net invalid"), "{}", c.evidence);
        let c = canary(true);
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        assert!(c.evidence.contains("richards net 0.800x"), "{}", c.evidence);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// I6: item 12 checks every corpus run in every log (a log can hold several cargo runs),
    /// and needs a release and a debug run.
    #[test]
    fn corpus_checks_every_run() {
        let root = fake_repo("corpus");
        write(&root, "in/ci.json", &ci_json("success"));
        let item12 = |logs: &[String]| {
            let mut o = opts(&root);
            o.corpus.clear();
            for (i, log) in logs.iter().enumerate() {
                let p = format!("in/c{i}.log");
                write(&root, &p, log);
                o.corpus.push(root.join(p));
            }
            run(&o).unwrap().cells[11].0.clone()
        };
        let release = corpus_log("release", "abc123", true);
        let debug = corpus_log("test", "abc123", true);
        // A release run that is ok, then a debug run with another digest that FAILED.
        let bad_debug = corpus_log("test", "deadbeef", false);
        let c = item12(&[format!("{release}{bad_debug}")]);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        // A debug run with the right digest that FAILED.
        let c = item12(&[release.clone(), corpus_log("test", "abc123", false)]);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        // Release only.
        let c = item12(std::slice::from_ref(&release));
        assert_eq!(c.status, Status::Open, "{}", c.evidence);
        assert!(c.evidence.contains("debug"), "{}", c.evidence);
        // Release and debug, in two logs or in one.
        let c = item12(&[release.clone(), debug.clone()]);
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        let c = item12(&[format!("{release}{debug}")]);
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The output of a cargo invocation that stops at a compile error: no "Finished" line, no
    /// test result (real cargo text, crate name changed).
    const COMPILE_FAILURE: &str = "   Compiling ep9_frontend v0.0.0 (/w/crates/ep9_frontend)\n\
        error[E0425]: cannot find value `undefined_name` in this scope\n \
        --> crates/ep9_frontend/src/lib.rs:3:5\n  |\n3 |     undefined_name\n  \
        |     ^^^^^^^^^^^^^^ not found in this scope\n\n\
        For more information about this error, try `rustc --explain E0425`.\n\
        error: could not compile `ep9_frontend` (lib test) due to 1 previous error\n";

    /// A test binary that crashed after it printed the digest: no test result.
    const CRASH: &str = "error: test failed, to rerun pass `-p ep9_frontend --lib`\n\n\
        Caused by:\n  process didn't exit successfully: `/w/target/release/deps/ep9_frontend-1 \
        bytecodegen::corpus --ignored --nocapture` (signal: 11, SIGSEGV: invalid memory reference)\n";

    /// M2-P2-RF2-I9: a cargo or rustc failure anywhere in a corpus or nesting log rejects the
    /// log, also when it is appended after a good run (it prints no "Finished" line, so it
    /// would join that run).
    #[test]
    fn logs_with_a_cargo_failure_are_rejected() {
        let root = fake_repo("cargo-failure");
        write(&root, "in/ci.json", &ci_json("success"));
        let item = |n: usize, logs: &[String]| {
            let mut o = opts(&root);
            o.corpus.clear();
            for (i, log) in logs.iter().enumerate() {
                let p = format!("in/f{i}.log");
                write(&root, &p, log);
                o.corpus.push(root.join(p));
            }
            if n == 6 {
                write(&root, "in/nesting.log", &logs[0]);
            }
            run(&o).unwrap().cells[n].0.clone()
        };
        let item12 = |logs: &[String]| item(11, logs);
        // A good run prints report lines with "error" in them; they are no cargo failure.
        let report = "      568  early error (front end)\n           e.g. test262/test/a.js: 4:1: \
            SyntaxError: error: x\n     8662  early error (negative test, expected)\n";
        let release = corpus_log("release", "abc123", true)
            .replace("running 1 test\n", &format!("running 1 test\n{report}"));
        let debug = corpus_log("test", "abc123", true);
        let c = item12(&[release.clone(), debug.clone()]);
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        // A compile failure appended to release + debug in one log, or to one of two logs.
        let c = item12(&[format!("{release}{debug}{COMPILE_FAILURE}")]);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("error[E0425]"), "{}", c.evidence);
        let c = item12(&[release.clone(), format!("{debug}{COMPILE_FAILURE}")]);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        // A rerun that crashed after its digest: the crash line is named.
        let crashed = corpus_log("test", "abc123", true).replace(
            "test bytecodegen::corpus::corpus_digest ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored\n",
            CRASH,
        );
        let c = item12(&[release.clone(), format!("{debug}{crashed}")]);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("error: test failed"), "{}", c.evidence);
        // A panic line after a good run, with and without the thread id.
        for panic in [
            "thread 'main' (77865648) panicked at src/x.rs:1:1:",
            "thread 'main' panicked at src/x.rs:1:1:",
        ] {
            let c = item12(&[release.clone(), format!("{debug}\n{panic}\nboom\n")]);
            assert_eq!(c.status, Status::Fail, "{}", c.evidence);
            assert!(c.evidence.contains("panicked at"), "{}", c.evidence);
        }
        // Item 7: the nesting log holds two cargo runs (NESTING_CMD); a failure appended after
        // the good output rejects it too.
        let nesting = read(&root.join("in/nesting.log")).unwrap();
        let c = item(6, std::slice::from_ref(&nesting));
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        for bad in [COMPILE_FAILURE, CRASH] {
            let c = item(6, &[format!("{nesting}{bad}")]);
            assert_eq!(c.status, Status::Fail, "{}", c.evidence);
            assert!(c.evidence.contains("error"), "{}", c.evidence);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn arguments() {
        let a = |s: &[&str]| {
            parse_args(
                &s.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                PathBuf::new(),
            )
        };
        assert!(
            a(&[
                "--host", "x", "--out", "o", "--corpus", "a", "--corpus", "b"
            ])
            .is_ok_and(|o| o.corpus.len() == 2)
        );
        assert!(a(&["--host", "x"]).is_err());
        assert!(a(&["--host", "x", "--out", "o", "--ci-run", "12a"]).is_err());
        assert!(a(&["--host", "x", "--out", "o", "--what"]).is_err());
        assert!(a(&["--host"]).is_err());
    }
}
