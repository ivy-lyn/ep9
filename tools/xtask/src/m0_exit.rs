//! `cargo xtask m0-exit` (M0 exit check, docs/plan/09-milestones.md section 2.2): the table of
//! the 12 M0 exit items, PASS, FAIL, OPEN or NOT-RUN per item, for one host and for
//! `x86_64-pc-windows-msvc` from the jobs of a CI run, with the evidence of each cell.
//!
//! ```text
//! cargo xtask m0-exit --host <triple> --out m0-exit.md
//!     [--exec <json>]       ep9-test262 exec --json                     items 1, 3 (and 2, 3 on Windows)
//!     [--parse <json>]      ep9-test262 parse --json                    item 2
//!     [--exec-gc <json>]... ep9-test262 exec --gc-stress 1 ... --json   item 6 (one whole
//!                           run, or every shard of one `--gc-shard` split)
//!     [--cross <json>]      tools/bench/cross run (ep9-cross/v1)        item 4
//!     [--nesting <log>]     cargo test output of the nesting tests      item 7
//!     [--corpus <log>]...   cargo test output of the corpus digest test item 12 (release, debug)
//!     [--verdicts <md>]...  spike verdict tables (default: the newest
//!                           docs/measurements/*-m0-spike-review.md)     items 5, 11
//!     [--ci-run <id>] [--ci-run-json <file>]                            the Windows column
//!     [--no-build-check]    item 8 without `cargo check -p ep9_value`
//! cargo xtask m0-exit --gc-shard <k>/<n>   print the test262 path prefixes of GC-stress shard k
//!                                          of n (CI runs one shard per job)
//! ```
//!
//! The task reads its inputs; it runs no test suite. A missing input gives NOT-RUN with the
//! command that makes it. `--ci-run <id>` alone runs `gh run view <id> --json
//! jobs,headSha,databaseId,url`; `--ci-run-json <file>` reads a file in that shape instead (an
//! agent without gh, or a saved run). The Windows column reads NOT-RUN only when no run is given
//! or the run does not exist. Item 4's Windows cell is N/A by design (a gate-host speed
//! reading); N/A counts as a pass.
//!
//! | Item | Host column | Windows column (CI jobs `<job> (windows-x64)`) |
//! |---|---|---|
//! | 1 | exec JSON: full run (runs = the pin's count), wall time <= 600 s, 0 infra; the run names the committed expectation file of this tree, and its "fail" results against that file give 0 new failures and 0 stale lines (recomputed here, not the runner's diff) | step "test262 exec" green and <= 600 s; the file is committed |
//! | 2 | parse JSON: negatives all pass (the pin's count), positives >= 99.98% | step "test262 parse" green, and the Windows parse expectation file equals this host's |
//! | 3 | exec JSON: `language/` files >= 40% | step "test262 exec" green; passed = total (exec JSON) - the `test/language/` lines of the Windows expectation file |
//! | 4 | cross JSON: richards and deltablue vs QuickJS, net interval low end >= 0.6, valid rows (a gross fallback row, net invalid, gives OPEN), a gate host, not provisional; measured at HEAD, or at an ancestor with no change under `crates/` or `tools/bench/` since (else OPEN) | N/A by design (a gate-host speed reading; no Windows bench job) |
//! | 5 | verdicts: SP-GC, SP-VA, SP-Value, SP-Dispatch, SP-JIT-mem and Spike B GO (no open conditions); SP-tsrs-win recorded | the same verdicts, and these steps of the run green: SP-GC, SP-Dispatch, SP-JIT-mem and Spike B structural gates, ep9_heap barrier, cargo test --workspace, spike-native tests (ep9_vm) |
//! | 6 | GC JSON: the whole pinned corpus (files run + skipped = the pin's count; `skipped_paths` = the set of the committed skip list, and `skipped` = its size), `timeout_ms` <= 60000 and `feedback_threshold` 0 (GC_CMD), 0 infra; the run names the committed expectation file, and its "fail" results against that file without the skip list give 0 new failures (recomputed here). Several GC JSONs must be every shard of one `--gc-shard` split of this tree's test262 checkout, each once, with the same settings: each JSON's `run_paths` (every file it ran) and `skipped_paths` are exactly the files of its shard, so together they hold every test file once; they are merged and checked as one run | every job `test262 GC-stress (windows-x64, k/n)`, k = 1..n each once, with its step "test262 exec, GC-stress (S1) + eager feedback" (or, in runs before M5-P2, "test262 exec, GC-stress + eager feedback") green |
//! | 7 | nesting log: `every_form_is_gated`, `caps_hold_end_to_end`, `uncaught_error_reports_file_line_utf16_column` ok, each one in a cargo run that finished with exit status 0, no cargo failure line (`error[E`, `error:`, a panic) anywhere in it, and its 2 cargo runs (or more) each between a start line `m0-exit start: <label>` and an exit-status line `m0-exit status: 0`, with no run unfinished and no line outside a run | step "cargo test --workspace" green (it runs the three) |
//! | 8 | the asserts are in ep9_value's source, and `cargo check -p ep9_value --target <host>` passes | step "cargo build --workspace --all-targets" green |
//! | 9, 10 | docs/adr/README.md index: the ADR status of every row due at M0 (Accepted or Frozen; a Draft for "Draft M0" rows); H20 Frozen | the same |
//! | 11 | verdicts: SP-Unit GO | the same |
//! | 12 | corpus logs: every cargo run in every log (from its start line `m0-exit start: <label>` to its exit-status line) prints the digest of `expectations/bytecode-corpus.digest`, its test is ok and its exit status is 0; no run is unfinished, no line is outside a run, and no log has a cargo failure line (`error[E`, `error:`, a panic) anywhere; a release and a debug run are both given | step "bytecodegen corpus digest (release)" green |
//!
//! After the table, the report lists two tree checks (plan rules, not exit items, so they do not
//! change the exit code): stub markers in crates, spikes and tools, and a `fn bind_this` in
//! crates/ep9_vm/src/call.rs.
//!
//! Exit 0 when every cell is PASS or N/A (the M0 exit holds), 1 otherwise, 2 on a usage error
//! or an input that cannot be read. The JSON reader is the small one in `exit/json.rs`: xtask
//! has no JSON crate. The input readers and the cell type live in `crate::exit` (shared with
//! `m1-exit`; moved there in M4-P0 with no logic change).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

// Glob imports: the items and the tests below use these modules' items by their plain names, as
// before the move.
use crate::exit::adr::*;
use crate::exit::ci::*;
use crate::exit::cross::*;
use crate::exit::gc_shards::*;
use crate::exit::json::*;
use crate::exit::logs::*;
use crate::exit::report::*;
use crate::exit::t262::*;
use crate::exit::verdict::*;
use crate::exit::{git, read};

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

/// The counts of a pin. Tests also know the small fake checkout of their GC shard tests.
fn pin_counts_of(pin: &str) -> Option<&'static PinCounts> {
    #[cfg(test)]
    if let Some(c) = tests::FAKE_PIN_COUNTS
        .iter()
        .find(|c| pin.starts_with(c.pin_prefix))
    {
        return Some(c);
    }
    PIN_COUNTS.iter().find(|c| pin.starts_with(c.pin_prefix))
}

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
const NESTING_CMD: &str = "echo \"m0-exit start: depth_tests\" > nesting.log; \
     cargo test --locked -p ep9_frontend --lib depth_tests >> nesting.log 2>&1; \
     echo \"m0-exit status: $?\" >> nesting.log; \
     echo \"m0-exit start: nesting_caps cli\" >> nesting.log; \
     cargo test --locked -p ep9_cli --test nesting_caps --test cli >> nesting.log 2>&1; \
     echo \"m0-exit status: $?\" >> nesting.log";

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

/// The parsed command line.
#[derive(Debug, Default)]
struct Options {
    root: PathBuf,
    host: String,
    out: PathBuf,
    exec: Option<PathBuf>,
    parse: Option<PathBuf>,
    /// One whole-corpus GC-stress JSON, or the shard JSONs of one split (`--gc-shard`).
    exec_gc: Vec<PathBuf>,
    cross: Option<PathBuf>,
    nesting: Option<PathBuf>,
    corpus: Vec<PathBuf>,
    verdicts: Vec<PathBuf>,
    ci_run: Option<String>,
    ci_run_json: Option<PathBuf>,
    build_check: bool,
}

const USAGE: &str = "usage: cargo xtask m0-exit --host <triple> --out <file.md> [--exec <json>] \
     [--parse <json>] [--exec-gc <json>]... [--cross <json>] [--nesting <log>] [--corpus <log>]... \
     [--verdicts <md>]... [--ci-run <id>] [--ci-run-json <file>] [--no-build-check]\n   \
     or: cargo xtask m0-exit --gc-shard <k>/<n>";

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
            "--exec-gc" => o.exec_gc.push(PathBuf::from(val()?)),
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
    if args.first().map(String::as_str) == Some("--gc-shard") {
        let [_, spec] = args else {
            return Err(format!("--gc-shard takes one <k>/<n> value\n{USAGE}"));
        };
        for p in gc_shard_prefixes(&root.join("third_party/test262"), spec)? {
            println!("{p}");
        }
        return Ok(true);
    }
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
    /// Every cell is PASS (or N/A by design): the M0 exit holds.
    fn all_pass(&self) -> bool {
        all_ok(&self.cells)
    }
}

/// Everything the items read, loaded once.
struct Ctx<'a> {
    o: &'a Options,
    pin: Option<String>,
    pin_counts: Option<&'static PinCounts>,
    exec: Option<(PathBuf, Json)>,
    parse: Option<(PathBuf, Json)>,
    gc: Vec<(PathBuf, Json)>,
    cross: Option<(PathBuf, Json)>,
    nesting: Option<(PathBuf, String)>,
    corpus: Vec<(PathBuf, String)>,
    verdicts: BTreeMap<String, (String, PathBuf)>,
    adr: Vec<AdrRow>,
    ci: Result<Ci, String>,
}

fn run(o: &Options) -> Result<Report, String> {
    let read_one = |p: &PathBuf| -> Result<(PathBuf, Json), String> {
        Ok((
            p.clone(),
            parse_json(&read(p)?).map_err(|e| format!("{}: {e}", p.display()))?,
        ))
    };
    let read_json = |p: &Option<PathBuf>| p.as_ref().map(read_one).transpose();
    let pin = std::fs::read_to_string(o.root.join("third_party/test262.commit"))
        .ok()
        .map(|s| s.trim().to_string());
    let pin_counts = pin.as_deref().and_then(pin_counts_of);
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
        gc: o.exec_gc.iter().map(read_one).collect::<Result<_, _>>()?,
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
    let execs = ctx
        .exec
        .iter()
        .map(|e| ("--exec", e))
        .chain(ctx.gc.iter().map(|e| ("--exec-gc", e)));
    for (what, (p, j)) in execs {
        if j.get("command").and_then(Json::str) != Some("exec") {
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
    for (p, j) in ctx.exec.iter().chain(&ctx.gc).chain(&ctx.parse) {
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
    Ok(render(
        &ctx,
        &items.map(|(t, _)| t),
        cells,
        &tree_checks(&o.root),
    ))
}

fn render(
    ctx: &Ctx,
    titles: &[&str; 12],
    cells: Vec<(Cell, Cell)>,
    tree: &[(&str, Cell)],
) -> Report {
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
    let all_pass = all_ok(&cells);
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
            "every item PASS on both columns (N/A cells are by design): the M0 exit holds"
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
    md.push_str(
        "\n## Tree checks\n\nPlan rules for the exit tree (plan-v8 section 1 rule 4 and section 13 row 12), \
         not exit items: they do not change the result above. A finding is OPEN and names the file.\n\n",
    );
    for (what, c) in tree {
        let _ = writeln!(md, "- {what}: {} {}", c.status.name(), c.evidence);
    }
    Report {
        table,
        markdown: md,
        cells,
    }
}

// ---------------------------------------------------------------------------------------------
// Items.

const EXEC_CMD: &str = "cargo run --release --locked -p ep9_test262 -- exec --json exec.json";

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
            let (new, stale, basis) = match recheck(&c.o.root, j, &file, &BTreeSet::new()) {
                Ok(d) => {
                    if !d.new.is_empty() {
                        why.push(format!("new failures: {}", some_paths(&d.new)));
                    }
                    (
                        d.new.len().max(runner_new),
                        d.stale.len().max(runner_stale),
                        d.basis(&file),
                    )
                }
                Err(e) => {
                    status = Status::Fail;
                    why.push(e);
                    (
                        runner_new,
                        runner_stale,
                        "the runner's diff, not recomputed".to_string(),
                    )
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
        Status::NotApplicable,
        "by design: item 4 is a gate-host speed reading (10-quality section 3; the host column), and no CI job runs richards or deltablue on windows-x64 (plan-v8 V8-A22)",
    );
    (host, win)
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
    let host = match c.gc.as_slice() {
        [] => not_given("--exec-gc", GC_CMD),
        [(p, j)] => gc_cell(c, &p.display().to_string(), j),
        shards => match merge_gc_shards(&c.o.root, shards) {
            Ok((label, merged)) => gc_cell(c, &label, &merged),
            Err(e) => e,
        },
    };
    let win = with_ci(c, gc_ci_cell);
    (host, win)
}

/// The item 6 host cell of one whole-corpus GC-stress JSON (or the merge of its shards).
fn gc_cell(c: &Ctx, label: &str, j: &Json) -> Cell {
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
    let skip = committed(&c.o.root, "test262-gc-skip.txt").and_then(|p| expectation_set(&p));
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
                    basis = d.basis(&file);
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
            "{label}: --gc-stress {gc}, files {fp} / {ft}, infra {infra}, new failures {new} ({basis}); {skip_text}{}",
            if why.is_empty() {
                String::new()
            } else {
                format!(". {}", why.join("; "))
            }
        ),
    )
}

/// The item 6 Windows cell: every GC-stress shard job of windows-x64 in the run, k = 1..n each
/// once, with its GC-stress step green.
fn gc_ci_cell(ci: &Ci) -> Result<Cell, String> {
    let head = format!("{GC_JOB} ({CI_MATRIX}, ");
    let mut shards: Vec<(usize, usize, &Job)> = Vec::new();
    for job in &ci.jobs {
        let Some(spec) = job
            .name
            .strip_prefix(&head)
            .and_then(|r| r.strip_suffix(')'))
        else {
            continue;
        };
        let kn = spec
            .split_once('/')
            .and_then(|(k, n)| Some((k.trim().parse().ok()?, n.trim().parse().ok()?)));
        match kn {
            Some((k, n)) if (1..=n).contains(&k) => shards.push((k, n, job)),
            _ => {
                return Ok(cell(
                    Status::Fail,
                    format!("job \"{}\" does not name its shard as k/n", job.name),
                ));
            }
        }
    }
    let Some(&(_, n, _)) = shards.first() else {
        return Err(format!(
            "the run has no \"{head}k/n)\" job (the GC-stress shards of ci.yml)"
        ));
    };
    if shards.iter().any(|s| s.1 != n) {
        return Ok(cell(
            Status::Fail,
            format!("the {head}k/n) jobs name different n"),
        ));
    }
    let mut status = Status::Pass;
    let mut parts = Vec::new();
    let mut seconds = 0;
    let mut seen = BTreeSet::new();
    let mut step: &str = GC_STEP; // the name the run's jobs use (GC_STEPS)
    for (k, _, job) in &shards {
        if !seen.insert(*k) {
            status = Status::Fail;
            parts.push(format!("shard {k}/{n} has two jobs"));
            continue;
        }
        match job.steps.iter().find(|s| is_gc_step(&s.name)) {
            Some(s) => {
                step = &s.name;
                status = status.max(s.status());
                seconds += s.seconds().unwrap_or(0);
                if s.status() != Status::Pass {
                    let how = s.describe();
                    parts.push(format!("shard {k}/{n}: step \"{step}\" {how}"));
                }
            }
            None => {
                status = status.max(Status::NotRun);
                parts.push(format!("job \"{}\" has no step \"{GC_STEP}\"", job.name));
            }
        }
    }
    let missing: Vec<String> = (1..=n)
        .filter(|k| !seen.contains(k))
        .map(|k| format!("{k}/{n}"))
        .collect();
    if !missing.is_empty() {
        status = Status::Fail;
        parts.push(format!(
            "no job for shard(s) {}: the run does not cover the corpus",
            missing.join(", ")
        ));
    }
    let text = format!(
        "{} of {n} \"{head}k/n)\" jobs, step \"{step}\" ({GC_CMD} on the prefixes of `cargo xtask m0-exit --gc-shard k/{n}`; {seconds} s over all shards){}",
        seen.len(),
        if parts.is_empty() {
            ": all green".to_string()
        } else {
            format!(": {}", parts.join("; "))
        }
    );
    Ok(cell(status, text))
}

fn item7(c: &Ctx) -> (Cell, Cell) {
    let host = match &c.nesting {
        None => not_given("--nesting", NESTING_CMD),
        Some((p, log)) => {
            let mut status = Status::Pass;
            let mut parts = Vec::new();
            // A test passes only in a cargo run that finished with exit status 0 (C-G2).
            let (runs, _) = invocations(log);
            for t in NESTING_TESTS {
                let ok = format!("test {t} ... ok");
                let failed = format!("test {t} ... FAILED");
                let st = if runs
                    .iter()
                    .any(|r| r.ok() && r.body.iter().any(|l| *l == ok))
                {
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
            // NESTING_CMD: two cargo runs, each with its start and exit-status lines.
            match invocation_check(log, 2) {
                Ok(()) => parts.push(format!(
                    "{} cargo runs, each with its start line and exit status 0",
                    runs.len()
                )),
                Err(why) => {
                    status = Status::Fail;
                    parts.push(format!("{why} (make it with `{NESTING_CMD}`)"));
                }
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

const CORPUS_CMD: &str = "echo \"m0-exit start: corpus release\" > corpus-release.log; \
     cargo test -p ep9_frontend --release --locked --lib bytecodegen::corpus -- --ignored --nocapture >> corpus-release.log 2>&1; \
     echo \"m0-exit status: $?\" >> corpus-release.log (and the same without --release, into corpus-debug.log)";

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
            if let Err(why) = invocation_check(log, 1) {
                status = Status::Fail;
                parts.push(format!(
                    "{}: {why} (make it with `{CORPUS_CMD}`)",
                    p.display()
                ));
            }
            let runs = corpus_runs(log);
            for r in runs {
                let ok = !r.failed
                    && r.ok >= r.digests.len()
                    && !r.digests.is_empty()
                    && r.exit == Some(0);
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
                    "{} ({}): {}, test {}, exit status {}",
                    p.display(),
                    r.profile,
                    digests.join(", "),
                    if ok { "ok" } else { "not ok" },
                    r.exit.map_or("missing".to_string(), |e| e.to_string())
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

/// The stub markers of plan-v8 section 1 rule 4. Written in two parts, so this file does not
/// hold them.
const STUB_MARKERS: [&str; 2] = [concat!("m3", "-stub"), concat!("m2", "-stub")];

/// The tree checks of the report (plan rules, not exit items; GATES-2 task 6): no stub marker in
/// crates, spikes or tools (rule 4: its two `grep -rn <marker> crates spikes tools` print
/// nothing), and no `fn bind_this` in crates/ep9_vm/src/call.rs (section 13 row 12, the D14 gap).
fn tree_checks(root: &Path) -> Vec<(&'static str, Cell)> {
    let stubs = stub_markers(root, &STUB_MARKERS, &["crates", "spikes", "tools"]);
    let call = root.join("crates/ep9_vm/src/call.rs");
    let bind = match read(&call) {
        Err(e) => cell(Status::NotRun, e),
        Ok(text) => {
            let hits: Vec<String> = text
                .lines()
                .enumerate()
                .filter(|(_, l)| l.contains("fn bind_this"))
                .map(|(i, _)| format!("crates/ep9_vm/src/call.rs:{}", i + 1))
                .collect();
            if hits.is_empty() {
                cell(
                    Status::Pass,
                    "crates/ep9_vm/src/call.rs has no `fn bind_this`",
                )
            } else {
                cell(
                    Status::Open,
                    format!("`fn bind_this` is back: {}", hits.join(", ")),
                )
            }
        }
    };
    vec![
        ("stub markers (plan-v8 section 1 rule 4)", stubs),
        ("`fn bind_this` in call.rs (section 13 row 12)", bind),
    ]
}

/// The Windows cell from the CI run: NOT-RUN without a run; a missing job or step is NOT-RUN;
/// an input file that cannot be read is NOT-RUN with the error.
fn with_ci(c: &Ctx, f: impl FnOnce(&Ci) -> Result<Cell, String>) -> Cell {
    match &c.ci {
        Err(e) => cell(Status::NotRun, e.clone()),
        Ok(ci) => f(ci).unwrap_or_else(|e| cell(Status::NotRun, e)),
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
        write(
            &root,
            "crates/ep9_vm/src/call.rs",
            "pub fn try_bind_this() {}\n",
        );
        let head = commit_all(&root);
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
        write(&root, "in/cross.json", &cross_json(&head));
        write(
            &root,
            "in/nesting.log",
            "m0-exit start: depth_tests\ntest depth_tests::every_form_is_gated ... ok\n\
             test result: ok. 1 passed\nm0-exit status: 0\nm0-exit start: nesting_caps cli\n\
             test caps_hold_end_to_end ... ok\ntest uncaught_error_reports_file_line_utf16_column ... ok\n\
             m0-exit status: 0\n",
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

    /// `git add -A` and a commit in `root` (a new repo the first time); the new HEAD.
    fn commit_all(root: &Path) -> String {
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(root)
                .args([
                    "-c",
                    "user.name=m0-exit test",
                    "-c",
                    "user.email=m0-exit@test.invalid",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        if !root.join(".git").exists() {
            git(&["init", "-q"]);
        }
        git(&["add", "-A"]);
        git(&["commit", "-q", "--no-verify", "--allow-empty", "-m", "fake"]);
        git(&["rev-parse", "HEAD"])
    }

    /// A gate-host cross run that measured `commit`, every canary row valid.
    fn cross_json(commit: &str) -> String {
        format!(
            r#"{{"schema":"ep9-cross/v1","commit":{{"ep9":"{commit}"}},"provisional":false,"host":{{"gate_host":true}},
              "M":{{"ratios":[
                {{"workload":"richards","comparator":"qjs","metric":"net","void":false,"net":{{"median":1.39,"interval":[1.37,1.43]}}}},
                {{"workload":"deltablue","comparator":"qjs","metric":"net","void":false,"net":{{"median":1.56,"interval":[1.51,1.58]}}}},
                {{"workload":"richards","comparator":"node-jitless","metric":"net","void":false,"net":{{"median":0.1,"interval":[0.1,0.1]}}}}]}}}}"#
        )
    }

    /// The output of one corpus test run in the `profile` cargo profile, with its start line
    /// and its exit-status line (CORPUS_CMD).
    fn corpus_log(profile: &str, digest: &str, ok: bool) -> String {
        format!(
            "m0-exit start: corpus {profile}\n    Finished `{profile}` profile [optimized] target(s) in 1s\n     Running unittests src/lib.rs\n\
             running 1 test\ncorpus digest {digest} over 77183 units (compare across hosts and targets)\n{}",
            if ok {
                "test bytecodegen::corpus::corpus_digest ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored\n\
                 m0-exit status: 0\n"
            } else {
                "test bytecodegen::corpus::corpus_digest ... FAILED\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored\n\
                 error: test failed, to rerun pass `-p ep9_frontend --lib`\nm0-exit status: 101\n"
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
        let gc = |k: usize, n: usize| {
            let steps: Vec<String> = conclusion(GC_STEP)
                .map(|c| step(GC_STEP, c, "05:00", "35:00"))
                .into_iter()
                .collect();
            format!(
                r#"{{"name":"{GC_JOB} ({CI_MATRIX}, {k}/{n})","status":"completed","conclusion":"success","databaseId":8,"steps":[{}]}}"#,
                steps.join(",")
            )
        };
        format!(
            r#"{{"databaseId":39636336281,"headSha":"44c5dcb","url":"https://x","jobs":[{},{},{},{},{},{}]}}"#,
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
            gc(1, 2),
            gc(2, 2),
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
            exec_gc: vec![root.join("in/gc.json")],
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
        // Item 4 on Windows is N/A by design, and it counts as a pass.
        assert_eq!(s[3], ("PASS", "N/A"), "{}", r.markdown);
        // SP-Dispatch has open conditions and Spike B is not reviewed.
        assert_eq!(s[4], ("OPEN", "OPEN"), "{}", r.markdown);
        // Item 6 on Windows: both GC-stress shard jobs green.
        assert_eq!(s[5], ("PASS", "PASS"), "{}", r.markdown);
        assert_eq!(s[6], ("PASS", "PASS"), "{}", r.markdown);
        assert_eq!(s[7], ("NOT-RUN", "PASS"), "{}", r.markdown);
        // Items 9 and 10 read the real ADR index, which moves as ADRs are accepted: the same
        // repo-wide status in both columns, PASS or OPEN.
        for i in [8, 9] {
            assert_eq!(s[i].0, s[i].1, "{}", r.markdown);
            assert!(["PASS", "OPEN"].contains(&s[i].0), "{}", r.markdown);
        }
        // The fake repo has only the 2026-10-21 review, where SP-Unit is not reviewed.
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
        // A CI job's JSON names the file in the job's checkout (M3-P3-DRYRUN-CI): PASS, and the
        // diff is still recomputed here (a new failure is still FAIL).
        let ci_path = format!("/no-such-ci-root/Users/ep9/expectations/test262.{HOST}.jit.txt");
        let c = item1(&all, &ci_path);
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        assert!(c.evidence.contains("another checkout"), "{}", c.evidence);
        let c = item1(&more, &ci_path);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        // A file that exists on this host under another expectations/ dir is not this tree's.
        write(
            &root,
            &format!("elsewhere/expectations/test262.{HOST}.jit.txt"),
            "test/x.js\n",
        );
        let there = root.join(format!("elsewhere/expectations/test262.{HOST}.jit.txt"));
        let c = item1(&all, there.to_str().unwrap());
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        // Another name in a CI checkout is not this file.
        let c = item1(&all, "/no-such-ci-root/work/ep9/expectations/other.txt");
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
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
        let head = git(&root, &["rev-parse", "HEAD"]).unwrap();
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
                r#"{{"schema":"ep9-cross/v1","commit":{{"ep9":"{head}"}},"provisional":false,"host":{{"gate_host":true}},
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

    // -----------------------------------------------------------------------------------------
    // M3-P3 GATES-2.

    /// The pin of the small test262 checkout of [`shard_repo`] (27 test files).
    pub(super) const FAKE_PIN_COUNTS: &[PinCounts] = &[PinCounts {
        pin_prefix: "fa4e5ead",
        files: 27,
        runs: 54,
        negatives: 1,
    }];

    /// The test files of [`shard_repo`]'s checkout (27; a `_FIXTURE` file and a .txt file are
    /// added next to them and are not tests).
    const FAKE_T262: [&str; 27] = [
        "test/annexB/a1.js",
        "test/annexB/a2.js",
        "test/built-ins/Array/x1.js",
        "test/built-ins/Array/x2.js",
        "test/built-ins/Array/x3.js",
        "test/built-ins/Array/x4.js",
        "test/built-ins/Array/x5.js",
        "test/built-ins/Array/x6.js",
        "test/built-ins/Map/m1.js",
        "test/built-ins/Map/m2.js",
        "test/built-ins/Map/m3.js",
        "test/built-ins/Map/m4.js",
        "test/harness/h1.js",
        "test/harness/h2.js",
        "test/language/expr/e1.js",
        "test/language/expr/e2.js",
        "test/language/expr/e3.js",
        "test/language/expr/e4.js",
        "test/language/expr/e5.js",
        "test/language/expr/e6.js",
        "test/language/stmt/s1.js",
        "test/language/stmt/s2.js",
        "test/language/stmt/s3.js",
        "test/language/stmt/s4.js",
        "test/skip/one.js",
        "test/skip/two.js",
        "test/staging/st1.js",
    ];

    /// A fake repo with the small checkout (pin fa4e5ead): (root, test files, failing files).
    /// Every other file fails, and so does skip/one.js; the skip list is [`SKIP`].
    fn shard_repo(name: &str) -> (PathBuf, Vec<String>, BTreeSet<String>) {
        let root = fake_repo(name);
        write(&root, "third_party/test262.commit", "fa4e5ead00\n");
        for f in FAKE_T262 {
            write(&root, &format!("third_party/test262/{f}"), "// t\n");
        }
        write(
            &root,
            "third_party/test262/test/language/m_FIXTURE.js",
            "// not a test\n",
        );
        write(&root, "third_party/test262/test/language/notes.txt", "x\n");
        let files = test262_files(&root.join("third_party/test262")).unwrap();
        assert_eq!(files, FAKE_T262.map(String::from));
        let mut fails: BTreeSet<String> = files.iter().step_by(2).cloned().collect();
        fails.insert(SKIP[0].into());
        let mut text = "# f\n".to_string();
        for f in &fails {
            text.push_str(f);
            text.push('\n');
        }
        write(
            &root,
            &format!("expectations/test262.{HOST}.jit.txt"),
            &text,
        );
        commit_all(&root);
        (root, files, fails)
    }

    /// The GC-stress JSON of one shard (its `prefixes`) of [`shard_repo`]: the skip list skips
    /// its tests, and the files of `fails` fail; `used` is the expectation file it names.
    fn shard_json(
        files: &[String],
        fails: &BTreeSet<String>,
        prefixes: &[String],
        used: &str,
        extra_fail: Option<&str>,
    ) -> String {
        let mine = |f: &str| prefixes.iter().any(|p| f.starts_with(p.as_str()));
        let skipped: Vec<&str> = SKIP.iter().copied().filter(|s| mine(s)).collect();
        let run: Vec<&str> = files
            .iter()
            .map(String::as_str)
            .filter(|f| mine(f) && !SKIP.contains(f))
            .collect();
        gc_json(&run, &skipped, fails, used, extra_fail)
    }

    /// A GC-stress JSON of a filtered run (as the runner writes it): the files of `run` were run
    /// (`run_paths`), those of `skipped` skipped, and the files of `fails` (plus `extra_fail`)
    /// fail; `used` is the expectation file it names.
    fn gc_json(
        run: &[&str],
        skipped: &[&str],
        fails: &BTreeSet<String>,
        used: &str,
        extra_fail: Option<&str>,
    ) -> String {
        let mut failing: Vec<&str> = run.iter().copied().filter(|f| fails.contains(*f)).collect();
        failing.extend(extra_fail);
        let results: Vec<String> = failing
            .iter()
            .map(|p| format!(r#"{{"path":"{p}","status":"fail","cause":"x","detail":"d"}}"#))
            .collect();
        let list = |paths: &[&str]| -> String {
            paths
                .iter()
                .map(|p| json_str(p))
                .collect::<Vec<_>>()
                .join(",")
        };
        let mut sorted_run = run.to_vec();
        sorted_run.sort_unstable();
        format!(
            r#"{{"command":"exec","mode":"jit","target":"{HOST}","workers":4,"timeout_ms":60000,"gc_stress":1,"feedback_threshold":0,
            "skipped":{},"skipped_paths":[{}],"run_paths":[{}],"wall_seconds":100,"files":{{"passed":{},"total":{}}},"runs":{{"passed":1,"total":2}},
            "by_dir":{{}},"causes":[],"results":[{}],"expectations":{{"file":{},"new_failures":[],"now_passing":[]}}}}"#,
            skipped.len(),
            list(skipped),
            list(&sorted_run),
            run.len() - failing.len().min(run.len()),
            run.len(),
            results.join(","),
            json_str(used)
        )
    }

    /// GATES-2 task 1, M3-P3-GC-SHARD: the split covers every test file exactly once, for every
    /// n, with no empty shard; on the real checkout (when it is there) the 6 shards are even.
    #[test]
    fn gc_split_is_a_partition() {
        let check = |files: &[String], n: usize| {
            let split = gc_split(files, n).unwrap();
            assert_eq!(split.len(), n);
            let mut total = 0;
            for f in files {
                let owners = split
                    .iter()
                    .filter(|(pre, _)| pre.iter().any(|p| f.starts_with(p.as_str())))
                    .count();
                assert_eq!(owners, 1, "{f} is in {owners} shards of {n}");
            }
            for (pre, count) in &split {
                let mine = files
                    .iter()
                    .filter(|f| pre.iter().any(|p| f.starts_with(p.as_str())))
                    .count();
                assert_eq!(mine, *count);
                assert!(*count > 0);
                total += count;
            }
            assert_eq!(total, files.len());
            // C-G1: the members of each shard are the files that the runner selects with the
            // shard's prefixes (any prefix, `starts_with`).
            let members = shard_members(files, &split);
            for (k, (pre, _)) in split.iter().enumerate() {
                let selected: Vec<&str> = files
                    .iter()
                    .map(String::as_str)
                    .filter(|f| pre.iter().any(|p| f.starts_with(p.as_str())))
                    .collect();
                assert_eq!(members[k], selected, "shard {k} of {n}");
            }
            split
        };
        let files: Vec<String> = FAKE_T262.map(String::from).to_vec();
        for n in 1..=8 {
            check(&files, n);
        }
        assert!(gc_split(&files, 28).is_err());
        assert!(gc_split(&files, 0).is_err());
        // A file whose path is a prefix of another file's path is never split from it.
        let tricky: Vec<String> = [
            "test/a/b.js",
            "test/a/b.js.js",
            "test/a/c.js",
            "test/a/d.js",
        ]
        .map(String::from)
        .to_vec();
        check(&tricky, 3);
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/test262");
        if real.join("test").is_dir() {
            let files = test262_files(&real).unwrap();
            let split = check(&files, 6);
            let (lo, hi) = (
                split.iter().map(|s| s.1).min().unwrap(),
                split.iter().map(|s| s.1).max().unwrap(),
            );
            assert!(hi - lo <= files.len() / 24, "{lo}..{hi}");
        }
    }

    /// `--gc-shard <k>/<n>`: shard k of the split of the checkout; bad specs are usage errors.
    #[test]
    fn gc_shard_spec() {
        let (root, files, _) = shard_repo("gc-shard-spec");
        let t262 = root.join("third_party/test262");
        let split = gc_split(&files, 3).unwrap();
        for k in 1..=3 {
            assert_eq!(
                gc_shard_prefixes(&t262, &format!("{k}/3")).unwrap(),
                split[k - 1].0
            );
        }
        for bad in ["0/3", "4/3", "1/33", "a/3", "3", "1/0", ""] {
            assert!(gc_shard_prefixes(&t262, bad).is_err(), "{bad}");
        }
        assert!(gc_shard_prefixes(&root.join("no-checkout"), "1/3").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// M3-P3-GC-SHARD: item 6 reads the shard JSONs of one split as one whole-corpus run. A
    /// missing, doubled or foreign shard is FAIL, never PASS.
    #[test]
    fn gc_shards_merge_into_one_run() {
        let (root, files, fails) = shard_repo("gc-shards");
        write(&root, "in/ci.json", &ci_json("success"));
        let committed = root.join(format!("expectations/test262.{HOST}.jit.txt"));
        let committed = committed.to_str().unwrap().to_string();
        let split = gc_split(&files, 3).unwrap();
        let shard = |k: usize, used: &str, extra: Option<&str>| {
            shard_json(&files, &fails, &split[k].0, used, extra)
        };
        let item6 = |jsons: &[String]| {
            let mut o = opts(&root);
            o.exec_gc.clear();
            for (i, j) in jsons.iter().enumerate() {
                let p = format!("in/gc-{i}.json");
                write(&root, &p, j);
                o.exec_gc.push(root.join(p));
            }
            run(&o).unwrap().cells[5].0.clone()
        };
        // all[k] is shard k + 1. All 3, in any order: PASS, as one run of the 27 files.
        let all: Vec<String> = (0..3).map(|k| shard(k, &committed, None)).collect();
        let c = item6(&[all[2].clone(), all[0].clone(), all[1].clone()]);
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        assert!(c.evidence.contains("3 shards"), "{}", c.evidence);
        assert!(c.evidence.contains("in/gc-0.json = 3/3"), "{}", c.evidence);
        // A missing shard: FAIL.
        let c = item6(&[all[2].clone(), all[0].clone()]);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("missing"), "{}", c.evidence);
        // One shard twice (and one missing): FAIL.
        let c = item6(&[all[0].clone(), all[0].clone(), all[1].clone()]);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("both shard"), "{}", c.evidence);
        // A shard with a result for a test of another shard: that test is not in its run_paths.
        // FAIL.
        let other = split[1].0[0].clone();
        let stray = files
            .iter()
            .find(|f| f.starts_with(other.as_str()))
            .unwrap();
        let c = item6(&[
            shard(0, &committed, Some(stray)),
            all[1].clone(),
            all[2].clone(),
        ]);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("did not run"), "{}", c.evidence);
        // Another timeout in one shard: two runs, FAIL.
        let c = item6(&[
            all[0].replace(r#""timeout_ms":60000"#, r#""timeout_ms":600000"#),
            all[1].clone(),
            all[2].clone(),
        ]);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("timeout_ms"), "{}", c.evidence);
        // A new failure in one shard (inside it, so the shard is placed): FAIL.
        let own = files
            .iter()
            .find(|f| {
                split[0].0.iter().any(|p| f.starts_with(p.as_str()))
                    && !fails.contains(*f)
                    && !SKIP.contains(&f.as_str())
            })
            .unwrap();
        let c = item6(&[
            shard(0, &committed, Some(own)),
            all[1].clone(),
            all[2].clone(),
        ]);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("new failures 1"), "{}", c.evidence);
        // CI shards name the file in their own checkout (Linux and Windows forms): PASS.
        for ci_path in [
            format!("/no-such-ci-root/work/ep9/expectations/test262.{HOST}.jit.txt"),
            format!("\\\\?\\Q:\\no-such-ci-root\\ep9\\expectations/test262.{HOST}.jit.txt"),
        ] {
            let c = item6(&[
                shard(0, &ci_path, None),
                shard(1, &ci_path, None),
                shard(2, &ci_path, None),
            ]);
            assert_eq!(c.status, Status::Pass, "{}", c.evidence);
            assert!(c.evidence.contains("another checkout"), "{}", c.evidence);
        }
        // No checkout to check the split against: OPEN.
        std::fs::remove_dir_all(root.join("third_party/test262")).unwrap();
        let c = item6(&all);
        assert_eq!(c.status, Status::Open, "{}", c.evidence);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// M3-P3 fix round 2, C-G1: item 6 checks the exact paths of each shard (`run_paths` plus
    /// `skipped_paths`), not only its file count and its non-passing paths. A shard that ran a
    /// passing test of another shard in place of one of its own gave PASS before.
    #[test]
    fn gc_shard_membership_is_exact() {
        let (root, files, fails) = shard_repo("gc-shard-members");
        write(&root, "in/ci.json", &ci_json("success"));
        let committed = root.join(format!("expectations/test262.{HOST}.jit.txt"));
        let committed = committed.to_str().unwrap().to_string();
        let split = gc_split(&files, 3).unwrap();
        let item6 = |jsons: &[String]| {
            let mut o = opts(&root);
            o.exec_gc.clear();
            for (i, j) in jsons.iter().enumerate() {
                let p = format!("in/gc-{i}.json");
                write(&root, &p, j);
                o.exec_gc.push(root.join(p));
            }
            run(&o).unwrap().cells[5].0.clone()
        };
        let in_shard = |k: usize, f: &str| split[k].0.iter().any(|p| f.starts_with(p.as_str()));
        let shard_run = |k: usize| -> Vec<&str> {
            files
                .iter()
                .map(String::as_str)
                .filter(|f| in_shard(k, f) && !SKIP.contains(f))
                .collect()
        };
        let shard_skip =
            |k: usize| -> Vec<&str> { SKIP.iter().copied().filter(|f| in_shard(k, f)).collect() };
        let good = |k: usize| gc_json(&shard_run(k), &shard_skip(k), &fails, &committed, None);
        let c = item6(&[good(0), good(1), good(2)]);
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        // Shard 1 runs a passing test of shard 2 in place of a passing test of its own: the
        // file count and the failures are the same, but the own test ran nowhere. FAIL.
        let passing = |k: usize| {
            shard_run(k)
                .into_iter()
                .find(|f| !fails.contains(*f))
                .unwrap()
        };
        let (own, other) = (passing(0), passing(1));
        let mut swapped: Vec<&str> = shard_run(0).into_iter().filter(|f| *f != own).collect();
        swapped.push(other);
        let overlap = gc_json(&swapped, &shard_skip(0), &fails, &committed, None);
        let c = item6(&[overlap, good(1), good(2)]);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("not shards of one"), "{}", c.evidence);
        // A shard JSON with no run_paths (a runner before C-G1) cannot be placed: FAIL.
        let old = good(0).replace(r#""run_paths":["#, r#""older_paths":["#);
        let c = item6(&[old, good(1), good(2)]);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("run_paths"), "{}", c.evidence);
        // run_paths that disagree with the file count, or a result outside run_paths: FAIL.
        let mut short_run = shard_run(0);
        short_run.pop();
        let short = gc_json(&short_run, &shard_skip(0), &fails, &committed, None).replace(
            &format!(r#""total":{}"#, short_run.len()),
            &format!(r#""total":{}"#, short_run.len() + 1),
        );
        let c = item6(&[short, good(1), good(2)]);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        let stray = gc_json(
            &shard_run(0),
            &shard_skip(0),
            &fails,
            &committed,
            Some(other),
        );
        let c = item6(&[stray, good(1), good(2)]);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// V8-A22: the item 6 Windows cell reads the windows-x64 GC-stress shard jobs of the run.
    #[test]
    fn windows_gc_shards_from_the_run() {
        let ci = |text: &str| Ci::from_json(&parse_json(text).unwrap()).unwrap();
        let c = gc_ci_cell(&ci(&ci_json("success"))).unwrap();
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        assert!(c.evidence.contains("2 of 2"), "{}", c.evidence);
        // A shard step that failed, or is not finished.
        let failed = ci_json_with(|n| Some(if n == GC_STEP { "failure" } else { "success" }));
        assert_eq!(gc_ci_cell(&ci(&failed)).unwrap().status, Status::Fail);
        let running = ci_json("success").replace(
            r#""status":"completed","conclusion":"success","number":1,"startedAt":"2026-10-21T23:05:00Z","completedAt":"2026-10-21T23:35:00Z""#,
            r#""status":"in_progress","conclusion":"","number":1,"startedAt":"2026-10-21T23:05:00Z","completedAt":"0001-01-01T00:00:00Z""#,
        );
        assert_ne!(running, ci_json("success"));
        assert_eq!(gc_ci_cell(&ci(&running)).unwrap().status, Status::NotRun);
        // A missing shard job: FAIL, never PASS.
        let mut run = ci(&ci_json("success"));
        run.jobs.retain(|j| !j.name.ends_with("2/2)"));
        let c = gc_ci_cell(&run).unwrap();
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("2/2"), "{}", c.evidence);
        // Two jobs for one shard, or jobs that name different n: FAIL.
        let mut run = ci(&ci_json("success"));
        run.jobs.iter_mut().for_each(|j| {
            if j.name.ends_with("2/2)") {
                j.name = format!("{GC_JOB} ({CI_MATRIX}, 1/2)");
            }
        });
        assert_eq!(gc_ci_cell(&run).unwrap().status, Status::Fail);
        let mut run = ci(&ci_json("success"));
        run.jobs.iter_mut().for_each(|j| {
            if j.name.ends_with("2/2)") {
                j.name = format!("{GC_JOB} ({CI_MATRIX}, 2/3)");
            }
        });
        assert_eq!(gc_ci_cell(&run).unwrap().status, Status::Fail);
        // No GC-stress job for windows-x64 (a run of an older ci.yml): NOT-RUN in the cell.
        let mut run = ci(&ci_json("success"));
        run.jobs.retain(|j| !j.name.starts_with(GC_JOB));
        assert!(gc_ci_cell(&run).unwrap_err().contains("no"));
    }

    /// V10-D19: the shard step names S1 since M5-P2 (GC6). m0-exit reads that name and the one
    /// before it (runs of an older ci.yml still read), names the step the run used, and gives
    /// NOT-RUN for any other name.
    #[test]
    fn windows_gc_shards_read_the_s1_and_the_older_step_name() {
        let ci = |text: &str| Ci::from_json(&parse_json(text).unwrap()).unwrap();
        assert!(GC_STEP.contains("S1") && GC_STEPS[0] == GC_STEP);
        let older = GC_STEPS[1];
        assert_eq!(older, "test262 exec, GC-stress + eager feedback");
        let new = ci_json("success");
        assert!(new.contains(GC_STEP));
        let c = gc_ci_cell(&ci(&new)).unwrap();
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        assert!(
            c.evidence.contains(&format!("step \"{GC_STEP}\" (")),
            "{}",
            c.evidence
        );
        let old = new.replace(GC_STEP, older);
        assert!(!old.contains(GC_STEP));
        let c = gc_ci_cell(&ci(&old)).unwrap();
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        assert!(c.evidence.contains("2 of 2"), "{}", c.evidence);
        assert!(
            c.evidence.contains(&format!("step \"{older}\" (")),
            "{}",
            c.evidence
        );
        // A failed shard step under the older name: FAIL, and the part names that step.
        let failed = ci_json_with(|n| Some(if n == GC_STEP { "failure" } else { "success" }))
            .replace(GC_STEP, older);
        let c = gc_ci_cell(&ci(&failed)).unwrap();
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(
            c.evidence.contains(&format!("1/2: step \"{older}\" ")),
            "{}",
            c.evidence
        );
        // Any other name (an S0 dispatch run names S0): the shard has no step m0-exit reads.
        let s0 = new.replace(GC_STEP, "test262 exec, GC-stress (S0) + eager feedback");
        let c = gc_ci_cell(&ci(&s0)).unwrap();
        assert_eq!(c.status, Status::NotRun, "{}", c.evidence);
        assert!(c.evidence.contains("has no step"), "{}", c.evidence);
    }

    /// GATES-2 task 1: item 4's Windows cell is N/A by design, and N/A counts as a pass.
    #[test]
    fn item4_windows_is_na_by_design() {
        let pass = || cell(Status::Pass, "p");
        let mut cells: Vec<(Cell, Cell)> = (0..12).map(|_| (pass(), pass())).collect();
        cells[3].1 = cell(Status::NotApplicable, "by design");
        assert!(all_ok(&cells));
        cells[3].1 = cell(Status::NotRun, "x");
        assert!(!all_ok(&cells));
        cells[3].1 = cell(Status::NotApplicable, "by design");
        cells[5].1 = cell(Status::Open, "x");
        assert!(!all_ok(&cells));
        assert_eq!(Status::NotApplicable.name(), "N/A");
        let root = fake_repo("item4-na");
        write(&root, "in/ci.json", &ci_json("success"));
        let r = run(&opts(&root)).unwrap();
        assert_eq!(r.cells[3].1.status, Status::NotApplicable);
        assert!(r.cells[3].1.evidence.contains("gate-host speed reading"));
        assert!(r.table.contains("| PASS | N/A |"), "{}", r.table);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// GATES-2 task 7 (V8-A14): item 4 is PASS only for a run of this tree, or of an ancestor
    /// with no change under crates/ or tools/bench/ since; else OPEN with the reason.
    #[test]
    fn cross_run_must_be_fresh() {
        let root = fake_repo("cross-fresh");
        let canary = |commit: &str| {
            cross_canary(
                Path::new("run.json"),
                &parse_json(&cross_json(commit)).unwrap(),
                &root,
            )
        };
        let measured = git(&root, &["rev-parse", "HEAD"]).unwrap();
        let c = canary(&measured);
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        assert!(c.evidence.contains("this tree (HEAD)"), "{}", c.evidence);
        let c = canary(&measured[..7]);
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        // A docs-only commit since: still PASS (an ancestor, no engine change).
        write(&root, "docs/note.md", "x\n");
        write(&root, "tools/xtask/x.rs", "x\n");
        commit_all(&root);
        let c = canary(&measured);
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        assert!(c.evidence.contains("an ancestor of HEAD"), "{}", c.evidence);
        // A change under crates/ or tools/bench/: OPEN, not this tree.
        for f in ["tools/bench/x.mjs", "crates/ep9_vm/src/x.rs"] {
            let before = git(&root, &["rev-parse", "HEAD"]).unwrap();
            write(&root, f, "x\n");
            commit_all(&root);
            let c = canary(&before);
            assert_eq!(c.status, Status::Open, "{f}: {}", c.evidence);
            assert!(c.evidence.contains("not this tree"), "{}", c.evidence);
        }
        let c = canary(&measured);
        assert_eq!(c.status, Status::Open, "{}", c.evidence);
        // An unknown commit, no commit, and a commit that is not an ancestor: OPEN.
        for commit in ["0123456789abcdef0123456789abcdef01234567", "unknown"] {
            let c = canary(commit);
            assert_eq!(c.status, Status::Open, "{commit}: {}", c.evidence);
        }
        let orphan = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@test.invalid",
                "commit-tree",
                "HEAD^{tree}",
                "-m",
                "orphan",
            ])
            .output()
            .unwrap();
        let orphan = String::from_utf8(orphan.stdout).unwrap();
        let c = canary(orphan.trim());
        assert_eq!(c.status, Status::Open, "{}", c.evidence);
        assert!(c.evidence.contains("not an ancestor"), "{}", c.evidence);
        // A fresh run that misses the gate still FAILs: freshness never lifts a FAIL.
        let head = git(&root, &["rev-parse", "HEAD"]).unwrap();
        let slow = cross_json(&head).replace("[1.37,1.43]", "[0.5,1.43]");
        let c = cross_canary(Path::new("run.json"), &parse_json(&slow).unwrap(), &root);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// GATES-2 task 2 (M2-P2-OPEN-I9-EXIT): items 7 and 12 need the exit-status line after each
    /// cargo run, all 0, at the end of the log; a killed run cannot pass.
    #[test]
    fn logs_need_their_exit_status() {
        let root = fake_repo("sentinel");
        write(&root, "in/ci.json", &ci_json("success"));
        let cells = |nesting: &str, release: &str| {
            write(&root, "in/nesting.log", nesting);
            write(&root, "in/corpus.log", release);
            let r = run(&opts(&root)).unwrap();
            (r.cells[6].0.clone(), r.cells[11].0.clone())
        };
        let nesting = read(&root.join("in/nesting.log")).unwrap();
        let release = corpus_log("release", "abc123", true);
        let (n, c) = cells(&nesting, &release);
        assert_eq!(n.status, Status::Pass, "{}", n.evidence);
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        // Killed before the status line: the output is all there, the status is not.
        let killed = |log: &str| log.replace("m0-exit status: 0\n", "");
        let last_killed = |log: &str| {
            let i = log.rfind("m0-exit status: 0").unwrap();
            log[..i].to_string()
        };
        for (what, nest, rel) in [
            ("no status at all", killed(&nesting), killed(&release)),
            (
                "the last run killed",
                last_killed(&nesting),
                last_killed(&release),
            ),
            (
                "a run killed by a signal (exit 137)",
                nesting.replacen("m0-exit status: 0", "m0-exit status: 137", 1),
                release.replace("m0-exit status: 0", "m0-exit status: 137"),
            ),
            (
                "output after the last status",
                format!("{nesting}test depth_tests::every_form_is_gated ... ok\n"),
                format!("{release}corpus digest abc123 over 77183 units\n"),
            ),
            (
                "a bad status line",
                nesting.replacen("m0-exit status: 0", "m0-exit status: x", 1),
                release.replace("m0-exit status: 0", "m0-exit status: x"),
            ),
        ] {
            let (n, c) = cells(&nest, &rel);
            assert_eq!(n.status, Status::Fail, "{what}: {}", n.evidence);
            assert_eq!(c.status, Status::Fail, "{what}: {}", c.evidence);
        }
        // The nesting log has 2 cargo runs: 1 run is not enough.
        let one = &nesting[..nesting.find("m0-exit start: nesting_caps").unwrap()];
        let (n, _) = cells(one, &release);
        assert_eq!(n.status, Status::Fail, "{}", n.evidence);
        assert!(n.evidence.contains("2 needed"), "{}", n.evidence);
        // A corpus run with no status of its own, followed by a good run, in one log: FAIL.
        let debug = corpus_log("test", "abc123", true);
        let (_, c) = cells(&nesting, &format!("{}{debug}", killed(&release)));
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("exit status missing"), "{}", c.evidence);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// M3-P3 fix round 2, C-G2: each cargo run of a log has its start line and its exit-status
    /// line. A retry that was killed between two good runs has a start and no status; it gave
    /// PASS before (the statuses were counted, not paired with the runs).
    #[test]
    fn an_interrupted_retry_is_rejected() {
        let root = fake_repo("interrupted");
        write(&root, "in/ci.json", &ci_json("success"));
        let cells = |nesting: &str, corpus: &str| {
            write(&root, "in/nesting.log", nesting);
            write(&root, "in/corpus.log", corpus);
            let mut o = opts(&root);
            o.corpus = vec![root.join("in/corpus.log")];
            let r = run(&o).unwrap();
            (r.cells[6].0.clone(), r.cells[11].0.clone())
        };
        let nesting = read(&root.join("in/nesting.log")).unwrap();
        let (release, debug) = (
            corpus_log("release", "abc123", true),
            corpus_log("test", "abc123", true),
        );
        let (n, c) = cells(&nesting, &format!("{release}{debug}"));
        assert_eq!(n.status, Status::Pass, "{}", n.evidence);
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        // A retry killed while cargo compiled: its start line and no status.
        let killed =
            "m0-exit start: retry\n   Compiling ep9_frontend v0.0.0 (/w/crates/ep9_frontend)\n";
        let second = nesting.find("m0-exit start: nesting_caps").unwrap();
        let nest = format!("{}{killed}{}", &nesting[..second], &nesting[second..]);
        let (n, c) = cells(&nest, &format!("{release}{killed}{debug}"));
        assert_eq!(n.status, Status::Fail, "{}", n.evidence);
        assert!(n.evidence.contains("no exit status"), "{}", n.evidence);
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(c.evidence.contains("no exit status"), "{}", c.evidence);
        // A status line with no start line, and output outside any run: FAIL.
        let no_start = nesting.replacen("m0-exit start: depth_tests\n", "", 1);
        let outside = format!("test caps_hold_end_to_end ... ok\n{nesting}");
        for (what, nest) in [("no start", no_start), ("outside", outside)] {
            let (n, _) = cells(&nest, &release);
            assert_eq!(n.status, Status::Fail, "{what}: {}", n.evidence);
        }
        // A required result only in a run that did not finish: FAIL (it is not that run's
        // result, even with good runs around it).
        let only_ok = "m0-exit start: x\ntest caps_hold_end_to_end ... ok\n\
            test uncaught_error_reports_file_line_utf16_column ... ok\n";
        let nest = format!("{}{only_ok}", &nesting[..second]);
        let (n, _) = cells(&nest, &release);
        assert_eq!(n.status, Status::Fail, "{}", n.evidence);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// GATES-2 task 6: the report lists stub markers and a `fn bind_this` in call.rs; they do
    /// not change the exit result.
    #[test]
    fn tree_checks_list_stubs_and_bind_this() {
        let root = fake_repo("tree-checks");
        let t = tree_checks(&root);
        assert_eq!(t.len(), 2);
        assert!(t.iter().all(|(_, c)| c.status == Status::Pass), "{t:?}");
        // An untracked file with a marker, and a tracked one.
        write(
            &root,
            "crates/ep9_vm/src/new.rs",
            &format!("#![allow(dead_code)] // {}\n", STUB_MARKERS[0]),
        );
        write(&root, "spikes/s/x.rs", &format!("// {}\n", STUB_MARKERS[1]));
        commit_all(&root);
        write(&root, "tools/t/y.rs", &format!("// {}\n", STUB_MARKERS[0]));
        write(
            &root,
            "crates/ep9_vm/src/call.rs",
            "pub fn try_bind_this() {}\npub(crate) fn bind_this() {}\n",
        );
        let t = tree_checks(&root);
        assert_eq!(t[0].1.status, Status::Open, "{t:?}");
        for f in [
            "crates/ep9_vm/src/new.rs:1",
            "spikes/s/x.rs:1",
            "tools/t/y.rs:1",
        ] {
            assert!(t[0].1.evidence.contains(f), "{f}: {}", t[0].1.evidence);
        }
        assert_eq!(t[1].1.status, Status::Open, "{t:?}");
        assert!(t[1].1.evidence.contains("call.rs:2"), "{}", t[1].1.evidence);
        // In the report, after the table; the result does not change.
        write(&root, "in/ci.json", &ci_json("success"));
        let r = run(&opts(&root)).unwrap();
        assert!(r.markdown.contains("## Tree checks"), "{}", r.markdown);
        assert!(r.markdown.contains("tools/t/y.rs:1"), "{}", r.markdown);
        std::fs::remove_file(root.join("crates/ep9_vm/src/call.rs")).unwrap();
        assert_eq!(tree_checks(&root)[1].1.status, Status::NotRun);
        // Outside a git work tree, every file is searched.
        std::fs::remove_dir_all(root.join(".git")).unwrap();
        let t = tree_checks(&root);
        assert_eq!(t[0].1.status, Status::Open, "{t:?}");
        assert!(
            t[0].1.evidence.contains("tools/t/y.rs:1"),
            "{}",
            t[0].1.evidence
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
