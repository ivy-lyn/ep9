//! `cargo xtask m1-exit` (M1 exit check, docs/plan/09-milestones.md section 2.2, M1 exit 1-10;
//! plan v9 section 9.4): the table of the 10 M1 exit items, PASS, FAIL, OPEN, N/A or NOT-RUN per
//! item, for one host and for `x86_64-pc-windows-msvc` from the jobs of a CI run.
//!
//! ```text
//! cargo xtask m1-exit --host <triple> --out m1-exit.md [--root <dir>]
//!     [--kernels <json>]      tools/bench/cross/kernels.mjs --check (ep9-kernels/v1)   item 1
//!     [--cross <json>]        tools/bench/cross/exit2.mjs --out (ep9-exit2/v1)         item 2
//!     [--exec <json>]         ep9-test262 exec --json                                 items 3, 4
//!     [--exec-gc <json>]...   ep9-test262 exec --gc-stress ... (S1 on) --json: one
//!                             whole run, or every shard of one --gc-shard split       item 4
//!     [--fuzz <json>]...      one ep9-fuzz/v1 record per counted campaign             item 5
//!     [--base <rev>]          the adr-lint range start (default m0-exit)              items 4, 6
//!     [--verdicts <md>]...    the Spike C and SP-Facts Verdict tables                 items 7, 10
//!     [--nesting <log>]       cargo test output of the nesting tests                  item 8
//!     [--ci-run <id>] [--ci-run-json <file>]                                          the Windows column
//! ```
//!
//! The task reads its inputs; it runs no test suite (item 6 runs `adr-lint`, a git and text
//! check). A missing input gives NOT-RUN, and the report prints one line per missing input with
//! the command that makes it. `--ci-run <id>` alone runs `gh run view <id> --json
//! jobs,headSha,databaseId,url`; `--ci-run-json <file>` reads a file in that shape instead.
//!
//! | Item | Host column | Windows column |
//! |---|---|---|
//! | 1 | kernels JSON of this `--host`: 15 of 15 kernels "runs, output equal", on a fresh tree: no uncommitted change, and the tree ids of `crates`, `tools/bench`, `Cargo.lock` and `rust-toolchain.toml` equal HEAD's (else OPEN) | job `bench-run (windows-x64)`, step `kernels` (month 5; NOT-RUN without it) |
//! | 2 | exit2 JSON: both readings `pass` (a `fail` is FAIL; marginal or no gate verdict is OPEN); a gate host, not provisional; fresh: its commit is HEAD, or an ancestor with no change in the four paths of item 1 since (else OPEN). On `x86_64-unknown-linux-gnu` without a gate host the cell reads OPEN "marginal (no gate host)" (V9-D31) | N/A by design |
//! | 3 | exec JSON: no GC-stress, 0 infra, `language/` >= 85% of the whole `test/language` dir of this tree's test262 checkout | step `test262 exec` green; passed = total (host exec JSON) - the `test/language/` lines of the Windows expectation file |
//! | 4 | GC JSONs (one run, or every shard of one split, each once): `gc_stress` > 0, `timeout_ms` <= 60000, `feedback_threshold` 0, S1 on (`"generational": true`), verify cadence 1 (`"verify_cadence": 1`; the row prints it), 0 infra, the whole corpus, the skipped tests = the committed skip list; the same pass set: with `--exec`, no test that passes there fails under GC-stress; without it, 0 new failures against the committed expectation file. The row prints the skip-list size and FAILs if it grew since `--base` without a `docs/decisions-log.md` line that names each added path (V9-A19, V9-A24) | every job `test262 GC-stress (windows-x64, k/n)` once, its step `test262 exec, GC-stress ...` green, and the step name says `S1` (else OPEN) |
//! | 5 | fuzz records: Fuzzilli >= 100 CPU-h and cargo-fuzz >= 24 CPU-h (counted records), 0 open unique crashes; a counted Fuzzilli record is fresh in `crates`, `Cargo.lock`, `rust-toolchain.toml` (V9-A30), a counted cargo-fuzz record in `crates/ep9_frontend`, `fuzz`, `Cargo.lock` (else OPEN) | repo-wide, as the host |
//! | 6 | `adr-lint --base <rev>` passes (all four parts) | repo-wide, as the host |
//! | 7 | the Spike C verdict is GO (no condition); H2, H14, H16, H17, H18, H19 Frozen, part by part (`exit::freeze`) | repo-wide, as the host |
//! | 8 | nesting log: the 7 `depth_tests::m1_*` tests, `caps_hold_end_to_end` and `every_m1_form_is_listed` ok, each in a cargo run that exited 0, no cargo failure line, every run with its start and exit-status lines | step `cargo test --workspace --locked` of `build + test (windows-x64)` green |
//! | 9 | H29-H34 Frozen, each part of its Freeze table that freezes by M1 exit (an append-only list is open, a later gate is not due; MC-w14-15) | repo-wide, as the host |
//! | 10 | the SP-Facts verdict is GO, or "GO, speed recorded" (09-milestones.md:241) | repo-wide, as the host |
//!
//! **Fuzz records** (`ep9-fuzz/v1`, one JSON object per campaign; plan v9 V9-D24, V9-A30):
//! `{"schema": "ep9-fuzz/v1", "tool": "fuzzilli" | "cargo-fuzz", "target": "<name>", "commit":
//! "<sha>", "cpu_hours": <n>, "counted": true | false, "crashes": [{"id": "<hash>", "state":
//! "open" | "fixed" | "duplicate" | "not-a-bug"}]}`. A record with `counted: false` only adds
//! its open crashes. The record holds hashes only, never an input (rule 14).
//!
//! The extra rows (no exit-code effect): the carried M0 item-5 cell (SP-Dispatch, read from the
//! newest `*-m0-spike-review.md`), the stub markers (`m2`, `m3`, `m4`, plan v9 rule 4) and the
//! ADR Counts line. The tree root is `--root`, or this workspace (as m0-exit). The shared readers
//! are in `crate::exit`.
//!
//! Exit 0 when every cell is PASS or N/A, 1 otherwise, 2 on a usage error or an input that
//! cannot be read.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::adr_lint;
use crate::exit::adr::counts_line;
use crate::exit::ci::{CI_MATRIX, CI_TRIPLE, Ci, gh_run};
use crate::exit::cross::fresh_since;
use crate::exit::freeze::frozen_by_m1;
use crate::exit::gc_shards::{GC_JOB, merge_gc_shards, test262_files};
use crate::exit::json::{Json, parse_json};
use crate::exit::logs::{cargo_failure, invocation_check, invocations};
use crate::exit::report::{Cell, Status, all_ok, cell, not_given, short, some_paths, stub_markers};
use crate::exit::t262::{committed, count, exec_health, expectation_set, pct, recheck};
use crate::exit::verdict::{newest_spike_review, verdict_table};
use crate::exit::{git, read};

/// The stub markers of plan v9 rule 4 (V9-D37). Written in two parts, so this file does not hold
/// them (m0-exit greps for the first two).
const STUB_MARKERS: [&str; 3] = [
    concat!("m2", "-stub"),
    concat!("m3", "-stub"),
    concat!("m4", "-stub"),
];

/// The dirs of the stub-marker row: the four dirs that plan v9 rule 4 greps (`fuzz/` is FE-FUZZ's
/// nightly workspace from M4-P2). m0-exit keeps its three dirs.
const STUB_DIRS: [&str; 4] = ["crates", "spikes", "tools", "fuzz"];

/// The 10 M1 exit items (09-milestones section 2.2, M1 exit), short titles.
const TITLES: [&str; 10] = [
    "All 8 v8-v7 kernels and Octane-7 run correctly",
    "Octane-7 >= 1.0x QuickJS; v8-v7 7-kernel geomean >= 0.75x node --jitless",
    "`language/` >= 85%",
    "Same pass set under GC-stress with S1 on; verifier clean",
    "Fuzzilli 100 CPU-h and front-end cargo-fuzz 24 CPU-h, 0 unique crashes",
    "No H-row change without an approved ADR amendment",
    "Spike C gate green; H2, H14 and H16 Frozen (H17-H19 at M1a)",
    "Conditional, assignment, unary, arrow and `**` chains pass cap-1 / cap+1",
    "H29-H34 Frozen",
    "SP-Facts green",
];

/// Item 1: the paths whose change makes a measured input stale (kernels.mjs FRESH_PATHS; the
/// GATES-3 tool note adds `Cargo.lock` and `rust-toolchain.toml` to the M0 rule). Items 2 and 5
/// use them too.
const FRESH_PATHS: [&str; 4] = ["crates", "tools/bench", "Cargo.lock", "rust-toolchain.toml"];

/// Item 1: the kernels (8 v8-v7 + 7 Octane).
const KERNELS: usize = 15;
const KERNEL_OK: &str = "runs, output equal";
const KERNELS_CMD: &str =
    "node tools/bench/cross/kernels.mjs --check target/release/ep9 --out kernels.json";

const CROSS_CMD: &str = "node tools/bench/cross/run.mjs --workloads exit2 --harness base ... --out run.json && node tools/bench/cross/exit2.mjs run.json --out exit2.json (on a gate host)";

const EXEC_CMD: &str = "cargo run --release --locked -p ep9_test262 -- exec --json exec.json";

/// Item 4: the GC-stress run; the runner's S1 flag (GC3-S1) and the JSON fields below come with
/// it.
const GC_CMD: &str = "cargo run --release --locked -p ep9_test262 -- exec --gc-stress 1 --feedback-threshold 0 --timeout 60 --skip-list expectations/test262-gc-skip.txt --json gc.json, with S1 on";
/// Item 4: the `--timeout` of [`GC_CMD`], in ms.
const GC_TIMEOUT_MS: f64 = 60_000.0;
/// Item 4: the GC JSON fields that say S1 was on and how often the verifier ran.
const GC_S1_KEY: &str = "generational";
const GC_CADENCE_KEY: &str = "verify_cadence";
/// Item 4: the GC skip list and the log that names each added path.
const GC_SKIP: &str = "test262-gc-skip.txt";
const DECISIONS_LOG: &str = "docs/decisions-log.md";

const FUZZ_HOW: &str = "one ep9-fuzz/v1 record per counted campaign (fuzz.yml, infra/fuzz)";
/// Item 5: CPU-hours per tool (09-milestones M1 exit 5).
const FUZZILLI_HOURS: f64 = 100.0;
const CARGO_FUZZ_HOURS: f64 = 24.0;
/// Item 5: the closed crash states.
const CLOSED: [&str; 3] = ["fixed", "duplicate", "not-a-bug"];

const VERDICTS_HOW: &str =
    "the Spike C and SP-Facts Verdict tables (a markdown table with Spike and Verdict columns)";

/// Item 7: the ADRs Frozen at M1 exit (H2, H14, H16) and at M1a (H17-H19).
const ITEM7_ADRS: [&str; 6] = ["H2", "H14", "H16", "H17", "H18", "H19"];
/// Item 9.
const ITEM9_ADRS: [&str; 6] = ["H29", "H30", "H31", "H32", "H33", "H34"];

/// Item 8: the tests of the 7 M1 forms (crates/ep9_frontend/src/depth_tests.rs `m1_*`,
/// crates/ep9_cli/tests/nesting_caps.rs).
const NESTING_TESTS: [&str; 9] = [
    "depth_tests::m1_conditional",
    "depth_tests::m1_conditional_consequent",
    "depth_tests::m1_assignment",
    "depth_tests::m1_unary",
    "depth_tests::m1_unary_typeof",
    "depth_tests::m1_arrow",
    "depth_tests::m1_exponent",
    "caps_hold_end_to_end",
    "every_m1_form_is_listed",
];
const NESTING_CMD: &str = "echo \"m0-exit start: depth_tests\" > nesting.log; \
     cargo test --locked -p ep9_frontend --lib depth_tests >> nesting.log 2>&1; \
     echo \"m0-exit status: $?\" >> nesting.log; \
     echo \"m0-exit start: nesting_caps cli\" >> nesting.log; \
     cargo test --locked -p ep9_cli --test nesting_caps --test cli >> nesting.log 2>&1; \
     echo \"m0-exit status: $?\" >> nesting.log";

/// The parsed command line.
#[derive(Debug, Default)]
struct Options {
    root: PathBuf,
    host: String,
    out: PathBuf,
    kernels: Option<PathBuf>,
    cross: Option<PathBuf>,
    exec: Option<PathBuf>,
    exec_gc: Vec<PathBuf>,
    fuzz: Vec<PathBuf>,
    base: String,
    verdicts: Vec<PathBuf>,
    nesting: Option<PathBuf>,
    ci_run: Option<String>,
    ci_run_json: Option<PathBuf>,
}

const USAGE: &str = "usage: cargo xtask m1-exit --host <triple> --out <file.md> [--root <dir>] \
     [--kernels <json>] [--cross <json>] [--exec <json>] [--exec-gc <json>]... [--fuzz <json>]... \
     [--base <rev>] [--verdicts <md>]... [--nesting <log>] [--ci-run <id>] [--ci-run-json <file>]";

fn parse_args(args: &[String], root: PathBuf) -> Result<Options, String> {
    let mut o = Options {
        root,
        base: "m0-exit".into(),
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
            "--root" => o.root = PathBuf::from(val()?),
            "--host" => o.host = val()?,
            "--out" => o.out = PathBuf::from(val()?),
            "--kernels" => o.kernels = Some(PathBuf::from(val()?)),
            "--cross" => o.cross = Some(PathBuf::from(val()?)),
            "--exec" => o.exec = Some(PathBuf::from(val()?)),
            "--exec-gc" => o.exec_gc.push(PathBuf::from(val()?)),
            "--fuzz" => o.fuzz.push(PathBuf::from(val()?)),
            "--base" => o.base = val()?,
            "--verdicts" => o.verdicts.push(PathBuf::from(val()?)),
            "--nesting" => o.nesting = Some(PathBuf::from(val()?)),
            "--ci-run" => o.ci_run = Some(val()?),
            "--ci-run-json" => o.ci_run_json = Some(PathBuf::from(val()?)),
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
    if o.base.is_empty() {
        return Err(format!("--base needs a revision\n{USAGE}"));
    }
    Ok(o)
}

/// Entry point: writes the report to `--out` and prints the table and the missing inputs.
pub fn main(args: &[String]) -> Result<bool, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    let opts = parse_args(args, root)?;
    let report = run(&opts)?;
    std::fs::write(&opts.out, &report.markdown)
        .map_err(|e| format!("cannot write {}: {e}", opts.out.display()))?;
    print!("{}", report.table);
    for (what, c) in &report.extra {
        println!(
            "extra row (no exit-code effect): {what}: {}",
            c.status.name()
        );
    }
    for m in &report.missing {
        println!("missing input: {m}");
    }
    println!("m1-exit: wrote {}", opts.out.display());
    Ok(all_ok(&report.cells))
}

/// The rendered result.
#[derive(Debug)]
struct Report {
    table: String,
    markdown: String,
    /// Per item (1-10): the host cell and the Windows cell.
    cells: Vec<(Cell, Cell)>,
    /// The extra rows: (what, cell). No exit-code effect.
    extra: Vec<(&'static str, Cell)>,
    /// One line per missing input: the flag, its items and the command that makes it.
    missing: Vec<String>,
}

/// Everything the items read, loaded once.
struct Ctx<'a> {
    o: &'a Options,
    kernels: Option<(PathBuf, Json)>,
    cross: Option<(PathBuf, Json)>,
    exec: Option<(PathBuf, Json)>,
    gc: Vec<(PathBuf, Json)>,
    fuzz: Vec<(PathBuf, Json)>,
    verdicts: BTreeMap<String, (String, PathBuf)>,
    nesting: Option<(PathBuf, String)>,
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
    let mut verdicts = BTreeMap::new();
    for f in &o.verdicts {
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
    let c = Ctx {
        o,
        kernels: read_json(&o.kernels)?,
        cross: read_json(&o.cross)?,
        exec: read_json(&o.exec)?,
        gc: o.exec_gc.iter().map(read_one).collect::<Result<_, _>>()?,
        fuzz: o.fuzz.iter().map(read_one).collect::<Result<_, _>>()?,
        verdicts,
        nesting: o
            .nesting
            .as_ref()
            .map(|p| Ok::<_, String>((p.clone(), read(p)?)))
            .transpose()?,
        ci,
    };
    // An exec JSON of another command or target is a usage error (as m0-exit).
    let execs = c
        .exec
        .iter()
        .map(|e| ("--exec", e))
        .chain(c.gc.iter().map(|e| ("--exec-gc", e)));
    for (what, (p, j)) in execs {
        if j.get("command").and_then(Json::str) != Some("exec") {
            return Err(format!(
                "{what} {}: not an ep9-test262 exec JSON",
                p.display()
            ));
        }
        let t = j.get("target").and_then(Json::str).unwrap_or("");
        if t != o.host {
            return Err(format!(
                "{what} {}: target {t:?}, but --host is {:?}",
                p.display(),
                o.host
            ));
        }
    }
    let mut missing = Vec::new();
    let mut miss = |given: bool, flag: &str, items: &str, how: &str| {
        if !given {
            missing.push(format!("{flag} ({items}): make it with `{how}`"));
        }
    };
    miss(o.kernels.is_some(), "--kernels", "item 1", KERNELS_CMD);
    miss(o.cross.is_some(), "--cross", "item 2", CROSS_CMD);
    miss(o.exec.is_some(), "--exec", "items 3, 4", EXEC_CMD);
    miss(!o.exec_gc.is_empty(), "--exec-gc", "item 4", GC_CMD);
    miss(!o.fuzz.is_empty(), "--fuzz", "item 5", FUZZ_HOW);
    miss(
        !o.verdicts.is_empty(),
        "--verdicts",
        "items 7, 10",
        VERDICTS_HOW,
    );
    miss(o.nesting.is_some(), "--nesting", "item 8", NESTING_CMD);
    if let Err(e) = &c.ci {
        missing.push(format!("the CI run (Windows column): {e}"));
    }
    type ItemFn = fn(&Ctx) -> (Cell, Cell);
    let items: [ItemFn; 10] = [
        item1, item2, item3, item4, item5, item6, item7, item8, item9, item10,
    ];
    let cells: Vec<(Cell, Cell)> = items.iter().map(|f| f(&c)).collect();
    Ok(render(&c, cells, extra_rows(&o.root), missing))
}

/// The Windows cell from the CI run: NOT-RUN without a run; a missing job or step is NOT-RUN.
fn with_ci(c: &Ctx, f: impl FnOnce(&Ci) -> Result<Cell, String>) -> Cell {
    match &c.ci {
        Err(e) => cell(Status::NotRun, e.clone()),
        Ok(ci) => f(ci).unwrap_or_else(|e| cell(Status::NotRun, e)),
    }
}

/// A repo-wide item: the Windows cell repeats the host cell.
fn repo_wide(host: Cell) -> (Cell, Cell) {
    let win = cell(
        host.status,
        format!("repo-wide, as the host column: {}", host.evidence),
    );
    (host, win)
}

fn because(why: &[String]) -> String {
    if why.is_empty() {
        String::new()
    } else {
        format!(". {}", why.join("; "))
    }
}

// ---------------------------------------------------------------------------------------------
// Items.

fn item1(c: &Ctx) -> (Cell, Cell) {
    let host = match &c.kernels {
        None => not_given("--kernels", KERNELS_CMD),
        Some((p, j)) => kernels_cell(&c.o.root, &c.o.host, p, j),
    };
    let win = with_ci(c, |ci| {
        let s = ci.step("bench-run", &["kernels"]).map_err(|e| {
            format!("{e}: the bench-run job comes in month 5 (plan v9 section 9.1)")
        })?;
        Ok(cell(
            s.status(),
            format!("step \"kernels\" {}", s.describe()),
        ))
    });
    (host, win)
}

fn kernels_cell(root: &Path, host: &str, p: &Path, j: &Json) -> Cell {
    if j.get("schema").and_then(Json::str) != Some("ep9-kernels/v1") {
        return cell(
            Status::Fail,
            format!("{}: not an ep9-kernels/v1 file", p.display()),
        );
    }
    let mut status = Status::Pass;
    let mut why = Vec::new();
    let target = j
        .path(&["host", "target"])
        .and_then(Json::str)
        .unwrap_or("");
    if target != host {
        status = Status::Fail;
        why.push(format!("measured on {target:?}, not on --host {host}"));
    }
    let rows = j.get("kernels").and_then(Json::arr).unwrap_or(&[]);
    let mut failing = Vec::new();
    let mut ok = 0;
    for r in rows {
        let id = r.get("id").and_then(Json::str).unwrap_or("?");
        let st = r.get("status").and_then(Json::str).unwrap_or("");
        if r.get("ok").and_then(Json::bool) == Some(true) && st == KERNEL_OK {
            ok += 1;
        } else {
            let blocker = r
                .get("blocker")
                .and_then(Json::str)
                .map_or(String::new(), |b| format!(", blocker {b}"));
            failing.push(format!("{id} ({st}{blocker})"));
        }
    }
    if rows.len() != KERNELS {
        status = Status::Fail;
        why.push(format!("{} kernels, not the {KERNELS}", rows.len()));
    }
    if !failing.is_empty() {
        status = Status::Fail;
        why.push(format!("not ok: {}", some_paths(&failing)));
    }
    // Fresh: no uncommitted change, and the tree ids at HEAD equal the measured ones.
    let commit = j.get("commit").and_then(Json::str).unwrap_or("unknown");
    let mut fresh = Vec::new();
    match j.get("dirty").and_then(Json::arr) {
        Some([]) => {}
        Some(d) => fresh.push(format!(
            "measured with uncommitted changes in {}",
            d.iter()
                .filter_map(Json::str)
                .collect::<Vec<_>>()
                .join(", ")
        )),
        None => fresh.push("no `dirty` list".into()),
    }
    for path in FRESH_PATHS {
        let theirs = j.path(&["tree", path]).and_then(Json::str);
        let ours = git(root, &["rev-parse", &format!("HEAD:{path}")]);
        if theirs.is_none() || theirs != ours.as_deref() {
            fresh.push(format!("{path} differs from HEAD"));
        }
    }
    let tree = if fresh.is_empty() {
        format!("fresh: {} unchanged at HEAD", FRESH_PATHS.join(", "))
    } else {
        status = status.max(Status::Open);
        format!("not fresh: {}", fresh.join("; "))
    };
    cell(
        status,
        format!(
            "{}: {ok}/{} {KERNEL_OK}; ep9 commit `{}`; {tree}{}",
            p.display(),
            rows.len(),
            short(commit),
            because(&why)
        ),
    )
}

fn item2(c: &Ctx) -> (Cell, Cell) {
    let host = match &c.cross {
        None => not_given("--cross", CROSS_CMD),
        Some((p, j)) => exit2_cell(&c.o.root, &c.o.host, p, j),
    };
    let win = cell(
        Status::NotApplicable,
        "by design: item 2 reads the gate hosts (plan v9 section 9.4 item 2: Windows N/A)",
    );
    (host, win)
}

fn exit2_cell(root: &Path, host: &str, p: &Path, j: &Json) -> Cell {
    match j.get("schema").and_then(Json::str) {
        Some("ep9-exit2/v1") => {}
        Some("ep9-cross/v1") => {
            return cell(
                Status::NotRun,
                format!(
                    "{}: an ep9-cross/v1 run; give the reading of `node tools/bench/cross/exit2.mjs {} --out exit2.json`",
                    p.display(),
                    p.display()
                ),
            );
        }
        s => {
            return cell(
                Status::Fail,
                format!("{}: not an ep9-exit2/v1 file (schema {s:?})", p.display()),
            );
        }
    }
    let mut status = Status::Pass;
    let mut parts = Vec::new();
    let readings = j.get("readings").and_then(Json::arr).unwrap_or(&[]);
    if readings.len() != 2 {
        status = Status::Fail;
        parts.push(format!("{} readings, not 2", readings.len()));
    }
    for r in readings {
        let label = r.get("label").and_then(Json::str).unwrap_or("?");
        let gate = r.get("gate").and_then(Json::str).unwrap_or("no gate");
        let st = match gate {
            "pass" => Status::Pass,
            "fail" => Status::Fail,
            _ => Status::Open,
        };
        status = status.max(st);
        let median = r
            .path(&["net", "median"])
            .and_then(Json::num)
            .map_or(String::new(), |m| format!(", net {m:.3}x"));
        parts.push(format!("{label}: {gate}{median}"));
    }
    let gate_host = j
        .path(&["host", "gate_host"])
        .and_then(Json::bool)
        .unwrap_or(false);
    let provisional = j.get("provisional").and_then(Json::bool).unwrap_or(true);
    if !gate_host {
        status = status.max(Status::Open);
        parts.push(if host == "x86_64-unknown-linux-gnu" {
            "marginal (no gate host): Linux x64 has no bare-metal gate host (V9-D31)".to_string()
        } else {
            "not a gate host".to_string()
        });
    }
    if provisional {
        status = status.max(Status::Open);
        parts.push("provisional run".into());
    }
    let commit = j.get("commit").and_then(Json::str).unwrap_or("unknown");
    let tree = match fresh_since(root, commit, &FRESH_PATHS) {
        Ok(t) => t,
        Err(e) => {
            status = status.max(Status::Open);
            e
        }
    };
    cell(
        status,
        format!(
            "{}: {}; ep9 commit `{}`: {tree}",
            p.display(),
            parts.join("; "),
            short(commit)
        ),
    )
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
        (Some((p, j)), Some((lp, lt))) => {
            let mut status = if lt > 0 && lp * 100 >= lt * 85 {
                Status::Pass
            } else {
                Status::Fail
            };
            let mut why = Vec::new();
            if j.get("gc_stress").and_then(Json::num).unwrap_or(0.0) != 0.0 {
                status = Status::Fail;
                why.push("a GC-stress run; give the default run".to_string());
            }
            let (infra, _, _) = exec_health(j);
            if infra > 0 {
                status = Status::Fail;
                why.push(format!("{infra} infrastructure errors"));
            }
            // The whole dir: as many language files as this tree's checkout has.
            match test262_files(&c.o.root.join("third_party/test262")) {
                Ok(files) => {
                    let n = files
                        .iter()
                        .filter(|f| f.starts_with("test/language/"))
                        .count() as u64;
                    if n != lt {
                        status = Status::Fail;
                        why.push(format!(
                            "{lt} language files, the checkout has {n} (a filtered run?)"
                        ));
                    }
                }
                Err(e) => {
                    status = status.max(Status::Open);
                    why.push(format!("the language file count cannot be checked: {e}"));
                }
            }
            cell(
                status,
                format!(
                    "{}: language/ {lp} / {lt} files = {:.2}% (>= 85%), infra {infra}{}",
                    p.display(),
                    pct(lp, lt),
                    because(&why)
                ),
            )
        }
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
            if lt > 0 && lp * 100 >= lt * 85 {
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
    let host = match c.gc.as_slice() {
        [] => not_given("--exec-gc", GC_CMD),
        [(p, j)] => gc_cell(c, &p.display().to_string(), j),
        shards => match merge_gc_shards(&c.o.root, shards) {
            Ok((label, merged)) => {
                // The merge keeps only what m0-exit reads: copy the S1 fields when all agree.
                let mut merged = merged;
                for key in [GC_S1_KEY, GC_CADENCE_KEY] {
                    let first = shards[0].1.get(key);
                    if shards.iter().all(|(_, j)| j.get(key) == first)
                        && let (Some(v), Json::Obj(m)) = (first, &mut merged)
                    {
                        m.push((key.to_string(), v.clone()));
                    }
                }
                gc_cell(c, &label, &merged)
            }
            Err(e) => e,
        },
    };
    let skip = skip_list_cell(c);
    let host = cell(
        host.status.max(skip.status),
        format!("{}; {}", host.evidence, skip.evidence),
    );
    let win = with_ci(c, gc_ci_cell);
    (host, win)
}

/// The skip-list part of item 4 (V9-A24): its size, and each path added since `--base` is named
/// in the decisions log.
fn skip_list_cell(c: &Ctx) -> Cell {
    let now = match committed(&c.o.root, GC_SKIP).and_then(|p| expectation_set(&p)) {
        Ok(s) => s,
        Err(e) => return cell(Status::Fail, format!("skip list: {e}")),
    };
    let base = &c.o.base;
    let Some(then) = git(
        &c.o.root,
        &["show", &format!("{base}:expectations/{GC_SKIP}")],
    ) else {
        return cell(
            Status::Open,
            format!(
                "skip list: {} paths; expectations/{GC_SKIP} cannot be read at {base}",
                now.len()
            ),
        );
    };
    let then: BTreeSet<String> = then
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();
    let added: Vec<&String> = now.difference(&then).collect();
    let log = read(&c.o.root.join(DECISIONS_LOG)).unwrap_or_default();
    let unnamed: Vec<String> = added
        .iter()
        .filter(|p| !log.contains(p.as_str()))
        .map(|p| p.to_string())
        .collect();
    let text = format!(
        "skip list expectations/{GC_SKIP}: {} paths ({base}: {}), {} added since",
        now.len(),
        then.len(),
        added.len()
    );
    if unnamed.is_empty() {
        cell(
            Status::Pass,
            if added.is_empty() {
                text
            } else {
                format!("{text}, each named in {DECISIONS_LOG}")
            },
        )
    } else {
        cell(
            Status::Fail,
            format!(
                "{text}; not named in {DECISIONS_LOG}: {}",
                some_paths(&unnamed)
            ),
        )
    }
}

/// The item 4 host cell of one whole-corpus GC-stress JSON (or the merge of its shards).
fn gc_cell(c: &Ctx, label: &str, j: &Json) -> Cell {
    let gc = j.get("gc_stress").and_then(Json::num).unwrap_or(0.0);
    let (infra, runner_new, _) = exec_health(j);
    let skipped = j.get("skipped").and_then(Json::num).unwrap_or(0.0) as usize;
    let (fp, ft) = count(j, &["files"]).unwrap_or((0, 0));
    let mut status = Status::Pass;
    let mut why = Vec::new();
    if gc <= 0.0 {
        status = Status::Fail;
        why.push("not a GC-stress run".to_string());
    }
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
    // S1 on, and the verifier after every collection (V9-A19: the row prints the cadence).
    let s1 = match j.get(GC_S1_KEY).and_then(Json::bool) {
        Some(true) => "S1 on".to_string(),
        Some(false) => {
            status = Status::Fail;
            why.push(format!("\"{GC_S1_KEY}\": false (S1 off)"));
            "S1 off".to_string()
        }
        None => {
            status = status.max(Status::Open);
            why.push(format!(
                "the JSON does not say S1 was on (no \"{GC_S1_KEY}\" field)"
            ));
            "S1 not recorded".to_string()
        }
    };
    let cadence = match j.get(GC_CADENCE_KEY).and_then(Json::num) {
        Some(1.0) => "verify cadence 1".to_string(),
        Some(n) => {
            status = Status::Fail;
            why.push(format!("verify cadence {n}, not 1 (V9-A19)"));
            format!("verify cadence {n}")
        }
        None => {
            status = status.max(Status::Open);
            why.push(format!(
                "the JSON does not say the verify cadence (no \"{GC_CADENCE_KEY}\" field)"
            ));
            "verify cadence not recorded".to_string()
        }
    };
    // The whole corpus of this tree's checkout: every file is run or skipped.
    let checkout = test262_files(&c.o.root.join("third_party/test262")).map(|f| f.len() as u64);
    match &checkout {
        Ok(n) if ft + skipped as u64 != *n => {
            status = Status::Fail;
            why.push(format!(
                "{ft} files run + {skipped} skipped, the checkout has {n} (a filtered run?)"
            ));
        }
        Ok(_) => {}
        Err(e) => {
            status = status.max(Status::Open);
            why.push(format!("the corpus size cannot be checked: {e}"));
        }
    }
    // The skipped tests are the committed skip list.
    let skip = committed(&c.o.root, GC_SKIP).and_then(|p| expectation_set(&p));
    match (&skip, j.get("skipped_paths").and_then(Json::arr)) {
        (Err(e), _) => {
            status = Status::Fail;
            why.push(e.clone());
        }
        (_, None) => {
            status = Status::Fail;
            why.push("no skipped_paths, so the skipped tests cannot be checked".into());
        }
        (Ok(skip), Some(a)) => {
            let paths: BTreeSet<String> = a
                .iter()
                .filter_map(|p| p.str().map(str::to_string))
                .collect();
            if paths != *skip || skipped != skip.len() {
                status = Status::Fail;
                let extra: Vec<String> = paths.difference(skip).cloned().collect();
                let gone: Vec<String> = skip.difference(&paths).cloned().collect();
                why.push(format!(
                    "skipped {skipped}, the skip list has {}{}{}",
                    skip.len(),
                    if extra.is_empty() {
                        String::new()
                    } else {
                        format!("; not in the list: {}", some_paths(&extra))
                    },
                    if gone.is_empty() {
                        String::new()
                    } else {
                        format!("; not skipped: {}", some_paths(&gone))
                    }
                ));
            }
        }
    }
    // The same pass set.
    let fails = |j: &Json| -> BTreeSet<String> {
        j.get("results")
            .and_then(Json::arr)
            .unwrap_or(&[])
            .iter()
            .filter(|r| r.get("status").and_then(Json::str) == Some("fail"))
            .filter_map(|r| r.get("path").and_then(Json::str).map(str::to_string))
            .collect()
    };
    let mut new = runner_new;
    let basis;
    match &c.exec {
        Some((ep, ej)) => {
            let (_, et) = count(ej, &["files"]).unwrap_or((0, 0));
            if ej.get("gc_stress").and_then(Json::num).unwrap_or(0.0) != 0.0 {
                status = Status::Fail;
                why.push(format!("--exec {} is a GC-stress run", ep.display()));
            }
            if let Ok(n) = &checkout
                && et != *n
            {
                status = Status::Fail;
                why.push(format!(
                    "--exec {} ran {et} files, the checkout has {n}",
                    ep.display()
                ));
            }
            let lost: Vec<String> = fails(j).difference(&fails(ej)).cloned().collect();
            if !lost.is_empty() {
                why.push(format!(
                    "pass without GC-stress, fail under it: {}",
                    some_paths(&lost)
                ));
            }
            new = lost.len();
            basis = format!("the pass set of --exec {}", ep.display());
        }
        None => {
            let file = format!("test262.{}.jit.txt", c.o.host);
            let skip = skip.clone().unwrap_or_default();
            match recheck(&c.o.root, j, &file, &skip) {
                Ok(d) => {
                    if !d.new.is_empty() {
                        why.push(format!("new failures: {}", some_paths(&d.new)));
                    }
                    new = new.max(d.new.len());
                    basis = format!("{} (no --exec given)", d.basis(&file));
                }
                Err(e) => {
                    status = Status::Fail;
                    why.push(e);
                    basis = "the runner's diff, not recomputed".into();
                }
            }
        }
    }
    if infra > 0 || new > 0 {
        status = Status::Fail;
        why.push(format!("{infra} infrastructure errors, {new} new failures"));
    }
    cell(
        status,
        format!(
            "{label}: --gc-stress {gc}, {s1}, {cadence}, files {fp} / {ft} + {skipped} skipped, infra {infra}, new failures {new} (against {basis}){}",
            because(&why)
        ),
    )
}

/// The item 4 Windows cell: every GC-stress shard job of windows-x64, k = 1..n each once, its
/// GC-stress step green, and that step's name says S1.
fn gc_ci_cell(ci: &Ci) -> Result<Cell, String> {
    let head = format!("{GC_JOB} ({CI_MATRIX}, ");
    let mut shards = Vec::new();
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
    if shards.iter().any(|s: &(usize, usize, _)| s.1 != n) {
        return Ok(cell(
            Status::Fail,
            format!("the {head}k/n) jobs name different n"),
        ));
    }
    let mut status = Status::Pass;
    let mut parts = Vec::new();
    let mut seen = BTreeSet::new();
    let mut names = BTreeSet::new();
    for (k, _, job) in &shards {
        if !seen.insert(*k) {
            status = Status::Fail;
            parts.push(format!("shard {k}/{n} has two jobs"));
            continue;
        }
        match job
            .steps
            .iter()
            .find(|s| s.name.starts_with("test262 exec, GC-stress"))
        {
            Some(s) => {
                status = status.max(s.status());
                names.insert(s.name.clone());
                if s.status() != Status::Pass {
                    parts.push(format!(
                        "shard {k}/{n}: step \"{}\" {}",
                        s.name,
                        s.describe()
                    ));
                }
            }
            None => {
                status = status.max(Status::NotRun);
                parts.push(format!(
                    "job \"{}\" has no step \"test262 exec, GC-stress ...\"",
                    job.name
                ));
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
    if names.iter().any(|s| !s.contains("S1")) {
        status = status.max(Status::Open);
        parts.push(
            "the GC-stress step name does not say S1, so the job record does not show S1 on (S1 becomes the CI GC-stress mode in month 5, V9-D27)"
                .into(),
        );
    }
    Ok(cell(
        status,
        format!(
            "{} of {n} \"{head}k/n)\" jobs{}",
            seen.len(),
            if parts.is_empty() {
                ": all green".to_string()
            } else {
                format!(": {}", parts.join("; "))
            }
        ),
    ))
}

fn item5(c: &Ctx) -> (Cell, Cell) {
    if c.fuzz.is_empty() {
        return repo_wide(not_given("--fuzz", FUZZ_HOW));
    }
    let mut status = Status::Pass;
    let mut why = Vec::new();
    let mut hours: BTreeMap<&str, f64> = BTreeMap::new();
    let mut open: Vec<String> = Vec::new();
    let mut runs = Vec::new();
    for (p, j) in &c.fuzz {
        if j.get("schema").and_then(Json::str) != Some("ep9-fuzz/v1") {
            status = Status::Fail;
            why.push(format!("{}: not an ep9-fuzz/v1 record", p.display()));
            continue;
        }
        let tool = j.get("tool").and_then(Json::str).unwrap_or("");
        let fresh_paths: &[&str] = match tool {
            "fuzzilli" => &["crates", "Cargo.lock", "rust-toolchain.toml"],
            "cargo-fuzz" => &["crates/ep9_frontend", "fuzz", "Cargo.lock"],
            _ => {
                status = Status::Fail;
                why.push(format!("{}: unknown tool {tool:?}", p.display()));
                continue;
            }
        };
        for cr in j.get("crashes").and_then(Json::arr).unwrap_or(&[]) {
            let state = cr.get("state").and_then(Json::str).unwrap_or("open");
            if !CLOSED.contains(&state) {
                open.push(format!(
                    "{} ({tool}, {state})",
                    cr.get("id").and_then(Json::str).unwrap_or("?")
                ));
            }
        }
        let counted = j.get("counted").and_then(Json::bool).unwrap_or(false);
        let h = j.get("cpu_hours").and_then(Json::num).unwrap_or(0.0);
        let commit = j.get("commit").and_then(Json::str).unwrap_or("unknown");
        if counted {
            match fresh_since(&c.o.root, commit, fresh_paths) {
                Ok(_) => {
                    *hours
                        .entry(if tool == "fuzzilli" {
                            "fuzzilli"
                        } else {
                            "cargo-fuzz"
                        })
                        .or_default() += h
                }
                Err(e) => {
                    status = status.max(Status::Open);
                    why.push(format!("{}: not counted: {e}", p.display()));
                }
            }
        }
        runs.push(format!(
            "{} {tool} {:.1} CPU-h at `{}`{}",
            p.display(),
            h,
            short(commit),
            if counted { "" } else { " (not counted)" }
        ));
    }
    for (tool, need) in [
        ("fuzzilli", FUZZILLI_HOURS),
        ("cargo-fuzz", CARGO_FUZZ_HOURS),
    ] {
        let got = hours.get(tool).copied().unwrap_or(0.0);
        if got < need {
            status = status.max(Status::Open);
            why.push(format!("{tool}: {got:.1} counted CPU-h of {need}"));
        }
    }
    if !open.is_empty() {
        status = Status::Fail;
        why.push(format!("open unique crashes: {}", some_paths(&open)));
    }
    repo_wide(cell(
        status,
        format!(
            "{}; Fuzzilli {:.1} / {FUZZILLI_HOURS} CPU-h, cargo-fuzz {:.1} / {CARGO_FUZZ_HOURS} CPU-h, {} open unique crashes{}",
            runs.join(", "),
            hours.get("fuzzilli").copied().unwrap_or(0.0),
            hours.get("cargo-fuzz").copied().unwrap_or(0.0),
            open.len(),
            because(&why)
        ),
    ))
}

fn item6(c: &Ctx) -> (Cell, Cell) {
    let host = match adr_lint::full(&c.o.root, &c.o.base) {
        Err(e) => cell(Status::NotRun, format!("adr-lint cannot run: {e}")),
        Ok(l) => {
            let findings: Vec<String> = l
                .text
                .lines()
                .filter(|t| {
                    !t.starts_with("adr-lint")
                        && !t.starts_with("UNUSED PIN")
                        && !t.starts_with("note: ")
                })
                .map(str::to_string)
                .collect();
            cell(
                if l.pass { Status::Pass } else { Status::Fail },
                format!(
                    "`cargo xtask adr-lint --base {}`: {}{}",
                    c.o.base,
                    l.summary,
                    if findings.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " ({} finding lines, first: {})",
                            findings.len(),
                            findings[0]
                        )
                    }
                ),
            )
        }
    };
    repo_wide(host)
}

/// The verdict of one spike: (status, text). A verdict with "condition" is OPEN.
fn spike_status(c: &Ctx, spike: &str) -> (Status, String) {
    let Some((v, file)) = c.verdicts.get(spike) else {
        return (Status::Open, format!("{spike}: no verdict"));
    };
    let l = v.to_ascii_lowercase();
    let status = if l.contains("no-go") || l.contains("fail") {
        Status::Fail
    } else if l.starts_with("go") && !l.contains("condition") {
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

/// The Frozen state of each ADR, part by part: OPEN while one is not Frozen.
fn frozen_cell(root: &Path, ids: &[&str]) -> (Status, String) {
    let mut status = Status::Pass;
    let mut parts = Vec::new();
    for id in ids {
        let (ok, t) = frozen_by_m1(root, id);
        if !ok {
            status = Status::Open;
        }
        parts.push(t);
    }
    (status, parts.join("; "))
}

fn item7(c: &Ctx) -> (Cell, Cell) {
    let (st, spike) = spike_status(c, "spike c");
    let (fst, frozen) = frozen_cell(&c.o.root, &ITEM7_ADRS);
    repo_wide(cell(st.max(fst), format!("{spike}; {frozen}")))
}

fn item8(c: &Ctx) -> (Cell, Cell) {
    let host = match &c.nesting {
        None => not_given("--nesting", NESTING_CMD),
        Some((p, log)) => {
            let mut status = Status::Pass;
            let mut parts = Vec::new();
            // A test passes only in a cargo run that finished with exit status 0.
            let (runs, _) = invocations(log);
            let mut not_ok = Vec::new();
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
                if st != Status::Pass {
                    not_ok.push(format!("{t} {}", st.name()));
                }
            }
            parts.push(if not_ok.is_empty() {
                format!("the {} tests ok", NESTING_TESTS.len())
            } else {
                format!("not ok: {}", not_ok.join(", "))
            });
            if log.contains("test result: FAILED") {
                status = Status::Fail;
                parts.push("a test binary FAILED".into());
            }
            if let Some(l) = cargo_failure(log) {
                status = Status::Fail;
                parts.push(format!("a cargo failure: {l:?}"));
            }
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
                "step \"cargo test --workspace --locked\" {} (it runs the depth_tests m1_* tests and nesting_caps)",
                s.describe()
            ),
        ))
    });
    (host, win)
}

fn item9(c: &Ctx) -> (Cell, Cell) {
    let (st, text) = frozen_cell(&c.o.root, &ITEM9_ADRS);
    repo_wide(cell(st, text))
}

fn item10(c: &Ctx) -> (Cell, Cell) {
    let (st, t) = spike_status(c, "sp-facts");
    repo_wide(cell(st, t))
}

// ---------------------------------------------------------------------------------------------

/// The extra rows (no exit-code effect): the carried M0 item-5 cell, the stub markers, the ADR
/// Counts line.
fn extra_rows(root: &Path) -> Vec<(&'static str, Cell)> {
    let m0_item5 = match newest_spike_review(root) {
        None => cell(
            Status::NotRun,
            "no docs/measurements/*-m0-spike-review.md in this tree",
        ),
        Some(f) => match read(&f) {
            Err(e) => cell(Status::NotRun, e),
            Ok(text) => sp_dispatch_cell(&text, &f),
        },
    };
    let counts = counts_line(root);
    let counts = if counts.is_empty() {
        cell(Status::NotRun, "docs/adr/README.md has no Counts line")
    } else {
        cell(Status::Pass, format!("docs/adr/README.md: {counts}"))
    };
    vec![
        ("carried M0 item 5 (SP-Dispatch condition 2)", m0_item5),
        (
            "stub markers (plan v9 rule 4)",
            stub_markers(root, &STUB_MARKERS, &STUB_DIRS),
        ),
        ("ADR Counts line", counts),
    ]
}

/// The carried M0 item-5 cell: the SP-Dispatch verdict of a spike review, by the word rule of
/// m0-exit's `spike_status` (a verdict with "condition" is OPEN). The last row wins, as in m0-exit
/// (a later table of the file overrides an earlier one).
fn sp_dispatch_cell(review: &str, file: &Path) -> Cell {
    let name = file
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let Some((_, v)) = verdict_table(review)
        .into_iter()
        .rfind(|(s, _)| s == "sp-dispatch")
    else {
        return cell(Status::Open, format!("sp-dispatch: no verdict ({name})"));
    };
    let l = v.to_ascii_lowercase();
    let st = if l.contains("no-go") || l.contains("fail") {
        Status::Fail
    } else if l.starts_with("go") && !l.contains("condition") {
        Status::Pass
    } else {
        Status::Open
    };
    cell(
        st,
        format!(
            "sp-dispatch: {v} ({name}); SP-Dispatch condition 2 on AMD EPYC 7763 (H15-Q5), carried from M0 (M3-P5-OPEN item 1)"
        ),
    )
}

fn render(
    c: &Ctx,
    cells: Vec<(Cell, Cell)>,
    extra: Vec<(&'static str, Cell)>,
    missing: Vec<String>,
) -> Report {
    let o = c.o;
    let head = git(&o.root, &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let mut table = String::new();
    let _ = writeln!(
        table,
        "| # | M1 exit item | {} | {CI_TRIPLE} (CI) |",
        o.host
    );
    let _ = writeln!(table, "|---|---|---|---|");
    for (i, (t, (h, w))) in TITLES.iter().zip(&cells).enumerate() {
        let _ = writeln!(
            table,
            "| {} | {t} | {} | {} |",
            i + 1,
            h.status.name(),
            w.status.name()
        );
    }
    let all_pass = all_ok(&cells);
    let mut md = String::from("# M1 exit check (`cargo xtask m1-exit`)\n\n");
    let _ = writeln!(
        md,
        "- Items: docs/plan/09-milestones.md section 2.2, M1 exit 1-10 (plan v9 section 9.4)."
    );
    let _ = writeln!(md, "- Host column: {}; tree at commit `{head}`.", o.host);
    let _ = writeln!(md, "- adr-lint range: `{}..HEAD`.", o.base);
    match &c.ci {
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
            "every item PASS on both columns (N/A cells are by design): the M1 exit holds"
        } else {
            "not every cell is PASS"
        }
    );
    md.push_str(&table);
    md.push_str("\n## Evidence\n");
    for (i, (t, (h, w))) in TITLES.iter().zip(&cells).enumerate() {
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
    md.push_str("\n## Missing inputs\n\n");
    if missing.is_empty() {
        md.push_str("None.\n");
    }
    for m in &missing {
        let _ = writeln!(md, "- {m}");
    }
    md.push_str("\n## Extra rows\n\nNot exit items: they do not change the result above.\n\n");
    for (what, c) in &extra {
        let _ = writeln!(md, "- {what}: {} {}", c.status.name(), c.evidence);
    }
    Report {
        table,
        markdown: md,
        cells,
        extra,
        missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    const HOST: &str = "aarch64-apple-darwin";

    /// The fixtures (plan v9 rule 24: no live ADR, README or expectation file is read).
    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/m1_exit")
            .join(name)
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn git_in(root: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "-c",
                "user.name=m1-exit test",
                "-c",
                "user.email=m1-exit@test.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "tag.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn commit_all(root: &Path, msg: &str) -> String {
        git_in(root, &["add", "-A"]);
        git_in(
            root,
            &["commit", "-q", "--no-verify", "--allow-empty", "-m", msg],
        );
        git_in(root, &["rev-parse", "HEAD"])
    }

    /// The test files of the small test262 checkout: 20 under `test/language/`, 4 others.
    fn t262_files() -> Vec<String> {
        let mut v: Vec<String> = (0..20)
            .map(|i| format!("test/language/l{i:02}.js"))
            .collect();
        v.extend((0..4).map(|i| format!("test/built-ins/b{i}.js")));
        v.sort();
        v
    }

    /// A fake repo (a git work tree) with the fixture ADR README and spike review, the four fresh
    /// paths, a small test262 checkout, the expectation and skip files; tagged `m0-exit`.
    fn fake_repo(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ep9-m1-exit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (from, to) in [
            ("adr-README.md", "docs/adr/README.md"),
            (
                "m0-spike-review.md",
                "docs/measurements/2026-10-26-m0-spike-review.md",
            ),
        ] {
            write(&root, to, &read(&fixture(from)).unwrap());
        }
        write(&root, "crates/ep9_vm/src/lib.rs", "// no marker\n");
        write(&root, "tools/bench/x.mjs", "// bench\n");
        write(&root, "Cargo.lock", "# lock\n");
        write(&root, "rust-toolchain.toml", "[toolchain]\n");
        write(&root, "docs/decisions-log.md", "# log\n");
        for f in t262_files() {
            write(&root, &format!("third_party/test262/{f}"), "// test\n");
        }
        write(
            &root,
            &format!("expectations/test262.{HOST}.jit.txt"),
            "test/language/l00.js\ntest/language/l01.js\ntest/built-ins/b0.js\n",
        );
        write(
            &root,
            "expectations/test262-gc-skip.txt",
            "# skip\ntest/language/l19.js\n",
        );
        git_in(&root, &["init", "-q"]);
        commit_all(&root, "fake");
        git_in(&root, &["tag", "m0-exit"]);
        root
    }

    fn opts(root: &Path) -> Options {
        Options {
            root: root.to_path_buf(),
            host: HOST.into(),
            out: root.join("out.md"),
            base: "m0-exit".into(),
            ..Options::default()
        }
    }

    /// Writes `j` as a JSON input file and returns its path.
    fn input(root: &Path, name: &str, j: &str) -> PathBuf {
        let p = root.join("inputs").join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, j).unwrap();
        p
    }

    fn results(fails: &[&str]) -> String {
        fails
            .iter()
            .map(|p| format!(r#"{{"path":"{p}","status":"fail"}}"#))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// With no input, every item that needs one is NOT-RUN and the report names each missing
    /// input with its command; the run is not a pass.
    #[test]
    fn missing_inputs_are_named() {
        let root = fake_repo("missing");
        let r = run(&opts(&root)).unwrap();
        assert_eq!(r.cells.len(), 10);
        assert!(!all_ok(&r.cells));
        assert_eq!(r.table.lines().count(), 12, "{}", r.table);
        for i in [0, 1, 2, 3, 4, 7] {
            assert_eq!(r.cells[i].0.status, Status::NotRun, "item {}", i + 1);
        }
        assert_eq!(r.cells[1].1.status, Status::NotApplicable);
        // 7 inputs and the CI run.
        assert_eq!(r.missing.len(), 8, "{:?}", r.missing);
        assert!(
            r.missing[0].starts_with(
                "--kernels (item 1): make it with `node tools/bench/cross/kernels.mjs"
            )
        );
        assert!(
            r.missing
                .iter()
                .any(|m| m.starts_with("--exec-gc (item 4)"))
        );
        assert!(
            r.markdown
                .contains("## Missing inputs\n\n- --kernels (item 1)")
        );
        // Item 6 runs adr-lint: the fake tree has no ADR with a Crates row.
        assert_eq!(r.cells[5].0.status, Status::NotRun, "{:?}", r.cells[5]);
        assert!(r.cells[5].0.evidence.contains("Crates"), "{:?}", r.cells[5]);
        // Items 7, 9, 10: no verdict, no ADR file.
        assert_eq!(r.cells[8].0.status, Status::Open);
        assert!(r.cells[8].0.evidence.contains("H29: no docs/adr/H29-*.md"));
        // The extra rows.
        assert_eq!(r.extra.len(), 3);
        assert_eq!(r.extra[0].1.status, Status::Open, "{:?}", r.extra[0]);
        assert_eq!(r.extra[1].1.status, Status::Pass, "{:?}", r.extra[1]);
        assert!(r.extra[2].1.evidence.contains("Counts on 2026-10-26"));
        // main() writes the file and returns "not a pass".
        let out = root.join("main.md");
        let args: Vec<String> = [
            "--root",
            root.to_str().unwrap(),
            "--host",
            HOST,
            "--out",
            out.to_str().unwrap(),
        ]
        .map(str::to_string)
        .to_vec();
        assert!(!main(&args).unwrap());
        assert!(read(&out).unwrap().starts_with("# M1 exit check"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The stub-marker row reads the m2, m3 and m4 markers, in the rule-4 dirs.
    #[test]
    fn the_stub_row_reads_three_markers() {
        let root = fake_repo("stubs");
        let r = run(&opts(&root)).unwrap();
        assert_eq!(
            r.extra[1].1.evidence,
            format!(
                "no `{}`, `{}` or `{}` line",
                STUB_MARKERS[0], STUB_MARKERS[1], STUB_MARKERS[2]
            )
        );
        for (i, f) in ["crates/a.rs", "spikes/b.rs", "tools/c.rs"]
            .iter()
            .enumerate()
        {
            write(
                &root,
                f,
                &format!("#![allow(dead_code)] // {}\n", STUB_MARKERS[i]),
            );
        }
        let c = &run(&opts(&root)).unwrap().extra[1].1;
        assert_eq!(c.status, Status::Open, "{c:?}");
        for f in ["crates/a.rs:1", "spikes/b.rs:1", "tools/c.rs:1"] {
            assert!(c.evidence.contains(f), "{f}: {}", c.evidence);
        }
        for f in ["crates/a.rs", "spikes/b.rs", "tools/c.rs"] {
            std::fs::remove_file(root.join(f)).unwrap();
        }
        write(&root, "fuzz/d.rs", &format!("// {}\n", STUB_MARKERS[2]));
        let c = &run(&opts(&root)).unwrap().extra[1].1;
        assert!(c.evidence.contains("fuzz/d.rs:1"), "{}", c.evidence);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Given inputs are read; one that cannot be read, or an exec JSON of another target, is an
    /// error (exit 2).
    #[test]
    fn inputs_are_read() {
        let root = fake_repo("inputs");
        let mut o = opts(&root);
        o.exec = Some(fixture("exec.json"));
        o.ci_run_json = Some(fixture("ci-run.json"));
        let r = run(&o).unwrap();
        // The fixture exec JSON has no by_dir counts.
        assert_eq!(r.cells[2].0.status, Status::Fail, "{:?}", r.cells[2]);
        assert!(r.cells[2].0.evidence.contains("no by_dir.language counts"));
        // The CI run has no test262 job.
        assert_eq!(r.cells[2].1.status, Status::NotRun, "{:?}", r.cells[2].1);
        assert!(r.markdown.contains("CI run 123 on commit `abc`"));
        o.ci_run = Some("7".into());
        assert!(run(&o).unwrap_err().contains("holds run 123"));
        o.ci_run = None;
        o.host = "x86_64-unknown-linux-gnu".into();
        assert!(run(&o).unwrap_err().contains("but --host is"));
        o.host = HOST.into();
        o.cross = Some(fixture("adr-README.md"));
        assert!(run(&o).unwrap_err().contains("JSON"));
        o.cross = Some(root.join("no-such.json"));
        assert!(run(&o).unwrap_err().contains("cannot read"));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn exec_json(lp: u64, lt: u64, total: u64, gc: f64, fails: &[&str]) -> String {
        format!(
            r#"{{"command":"exec","target":"{HOST}","gc_stress":{gc},"files":{{"passed":{},"total":{total}}},"by_dir":{{"language":{{"files":{{"passed":{lp},"total":{lt}}}}}}},"results":[{}]}}"#,
            total - fails.len() as u64,
            results(fails)
        )
    }

    #[test]
    fn item3_language_share() {
        let root = fake_repo("item3");
        let mut o = opts(&root);
        for (lp, lt, gc, want, text) in [
            (
                17,
                20,
                0.0,
                Status::Pass,
                "language/ 17 / 20 files = 85.00% (>= 85%)",
            ),
            (16, 20, 0.0, Status::Fail, "= 80.00%"),
            (
                19,
                19,
                0.0,
                Status::Fail,
                "19 language files, the checkout has 20",
            ),
            (20, 20, 1.0, Status::Fail, "a GC-stress run"),
        ] {
            o.exec = Some(input(&root, "exec.json", &exec_json(lp, lt, 24, gc, &[])));
            let c = &run(&o).unwrap().cells[2].0;
            assert_eq!(c.status, want, "{c:?}");
            assert!(c.evidence.contains(text), "{text}: {}", c.evidence);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn item8_reads_the_m1_tests() {
        let root = fake_repo("item8");
        let mut o = opts(&root);
        o.nesting = Some(fixture("nesting.log"));
        let c = &run(&o).unwrap().cells[7].0;
        assert_eq!(c.status, Status::Pass, "{c:?}");
        assert!(
            c.evidence.contains("the 9 tests ok, 2 cargo runs"),
            "{}",
            c.evidence
        );
        let log = read(&fixture("nesting.log")).unwrap();
        for (edit, want, text) in [
            (
                log.replace("test depth_tests::m1_exponent ... ok\n", ""),
                Status::NotRun,
                "depth_tests::m1_exponent NOT-RUN",
            ),
            (
                log.replace("m1_unary ... ok", "m1_unary ... FAILED"),
                Status::Fail,
                "depth_tests::m1_unary FAIL",
            ),
            (
                log.replacen("m0-exit status: 0", "m0-exit status: 101", 1),
                Status::Fail,
                "exited 101",
            ),
            (
                log.trim_end()
                    .trim_end_matches("m0-exit status: 0")
                    .to_string(),
                Status::Fail,
                "no exit status",
            ),
        ] {
            o.nesting = Some(input(&root, "nesting.log", &edit));
            let c = &run(&o).unwrap().cells[7].0;
            assert_eq!(c.status, want, "{text}: {c:?}");
            assert!(c.evidence.contains(text), "{text}: {}", c.evidence);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The Freeze-table form of README section 3.4, with a Contract part due at M1 exit.
    fn adr(id: &str, status: &str, table: bool) -> String {
        let t = if table {
            "\n## Freeze table\n\n| Part | What it holds | Freeze point | After |\n|---|---|---|---|\n\
             | Contract | rules | M1 exit | frozen |\n| Kinds (append-only) | list | None | append |\n\
             | Finalize (gated) | forms | The A4 gates (M8 exit) | frozen |\n"
        } else {
            ""
        };
        format!("# {id}\n\n| Field | Value |\n|---|---|\n| Status | {status} |\n{t}")
    }

    fn write_adrs(root: &Path, ids: &[&str], status: &str, table: bool) {
        for id in ids {
            let n: u32 = id[1..].parse().unwrap();
            write(
                root,
                &format!("docs/adr/H{n:02}-x.md"),
                &adr(id, status, table),
            );
        }
    }

    /// MC-w14-15: item 9 reads each Freeze-table part; a whole-row Frozen word does not pass.
    #[test]
    fn item9_reads_each_freeze_part() {
        let root = fake_repo("item9");
        for (status, want) in [
            ("Draft (2026-10-27)", Status::Open),
            ("Frozen (2026-12-01, M1 exit)", Status::Open),
            ("Frozen (contract; 2026-12-01, M1 exit)", Status::Pass),
        ] {
            write_adrs(&root, &ITEM9_ADRS, status, true);
            let (h, w) = &run(&opts(&root)).unwrap().cells[8];
            assert_eq!(h.status, want, "{status}: {h:?}");
            assert_eq!(w.status, want);
            assert!(
                h.evidence
                    .contains("open lists (an append is a fill): Kinds (append-only)"),
                "{}",
                h.evidence
            );
            assert!(
                h.evidence.contains("later parts: Finalize (gated)"),
                "{}",
                h.evidence
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn items7_and_10_read_verdicts_and_freezes() {
        let root = fake_repo("item7");
        let mut o = opts(&root);
        o.verdicts = vec![fixture("verdicts.md")];
        write_adrs(&root, &ITEM7_ADRS, "Frozen (2026-12-01, M1 exit)", false);
        let r = run(&o).unwrap();
        assert_eq!(r.cells[6].0.status, Status::Pass, "{:?}", r.cells[6]);
        assert!(
            r.cells[6]
                .0
                .evidence
                .starts_with("spike c: GO (verdicts.md); H2: Frozen")
        );
        assert_eq!(r.cells[9].0.status, Status::Pass, "{:?}", r.cells[9]);
        assert!(r.cells[9].0.evidence.contains("GO, speed recorded"));
        // H17 still Draft: OPEN; a verdict with a condition: OPEN.
        write(
            &root,
            "docs/adr/H17-x.md",
            &adr("H17", "Draft (2026-10-21)", false),
        );
        let v = read(&fixture("verdicts.md"))
            .unwrap()
            .replace("| GO |", "| GO with a condition |");
        o.verdicts = vec![input(&root, "v.md", &v)];
        let r = run(&o).unwrap();
        assert_eq!(r.cells[6].0.status, Status::Open);
        assert!(
            r.cells[6]
                .0
                .evidence
                .contains("H17: Draft (no Freeze table: the whole row is not Frozen)")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn kernels_json(
        root: &Path,
        target: &str,
        ok: usize,
        total: usize,
        tree: Option<&str>,
    ) -> String {
        let rows: Vec<String> = (0..total)
            .map(|i| {
                if i < ok {
                    format!(r#"{{"id":"k{i}","status":"runs, output equal","ok":true}}"#)
                } else {
                    format!(r#"{{"id":"k{i}","status":"fails","ok":false,"blocker":"RegExp"}}"#)
                }
            })
            .collect();
        let ids: Vec<String> = FRESH_PATHS
            .iter()
            .map(|p| {
                let id = tree.map_or_else(
                    || git_in(root, &["rev-parse", &format!("HEAD:{p}")]),
                    str::to_string,
                );
                format!(r#""{p}":"{id}""#)
            })
            .collect();
        format!(
            r#"{{"schema":"ep9-kernels/v1","commit":"{}","tree":{{{}}},"dirty":[],"host":{{"target":"{target}"}},"kernels":[{}]}}"#,
            git_in(root, &["rev-parse", "HEAD"]),
            ids.join(","),
            rows.join(",")
        )
    }

    #[test]
    fn item1_needs_15_kernels_on_a_fresh_tree() {
        let root = fake_repo("item1");
        let mut o = opts(&root);
        for (j, want, text) in [
            (
                kernels_json(&root, HOST, 15, 15, None),
                Status::Pass,
                "15/15 runs, output equal",
            ),
            (
                kernels_json(&root, HOST, 14, 15, None),
                Status::Fail,
                "not ok: k14 (fails, blocker RegExp)",
            ),
            (
                kernels_json(&root, HOST, 8, 8, None),
                Status::Fail,
                "8 kernels, not the 15",
            ),
            (
                kernels_json(&root, "x86_64-unknown-linux-gnu", 15, 15, None),
                Status::Fail,
                "not on --host",
            ),
            (
                kernels_json(&root, HOST, 15, 15, Some("0000")),
                Status::Open,
                "Cargo.lock differs from HEAD",
            ),
            (
                kernels_json(&root, HOST, 15, 15, None)
                    .replace(r#""dirty":[]"#, r#""dirty":["crates/x.rs"]"#),
                Status::Open,
                "uncommitted changes in crates/x.rs",
            ),
        ] {
            o.kernels = Some(input(&root, "kernels.json", &j));
            let c = &run(&o).unwrap().cells[0].0;
            assert_eq!(c.status, want, "{text}: {c:?}");
            assert!(c.evidence.contains(text), "{text}: {}", c.evidence);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    fn exit2_json(commit: &str, gates: [&str; 2], gate_host: bool) -> String {
        format!(
            r#"{{"schema":"ep9-exit2/v1","commit":"{commit}","host":{{"gate_host":{gate_host}}},"provisional":false,"readings":[{{"label":"Octane-7","gate":"{}","net":{{"median":1.1}}}},{{"label":"v8-v7","gate":"{}"}}]}}"#,
            gates[0], gates[1]
        )
    }

    #[test]
    fn item2_reads_the_exit2_readings() {
        let root = fake_repo("item2");
        let head = git_in(&root, &["rev-parse", "HEAD"]);
        let mut o = opts(&root);
        for (j, host, want, text) in [
            (
                exit2_json(&head, ["pass", "pass"], true),
                HOST,
                Status::Pass,
                "Octane-7: pass, net 1.100x; v8-v7: pass",
            ),
            (
                exit2_json(&head, ["pass", "fail"], true),
                HOST,
                Status::Fail,
                "v8-v7: fail",
            ),
            (
                exit2_json(&head, ["pass", "marginal"], true),
                HOST,
                Status::Open,
                "marginal",
            ),
            (
                exit2_json(&head, ["pass", "pass"], false),
                "x86_64-unknown-linux-gnu",
                Status::Open,
                "marginal (no gate host)",
            ),
            (
                exit2_json("0123abc", ["pass", "pass"], true),
                HOST,
                Status::Open,
                "is not in this repository",
            ),
            (
                r#"{"schema":"ep9-cross/v1"}"#.to_string(),
                HOST,
                Status::NotRun,
                "exit2.mjs",
            ),
        ] {
            o.host = host.into();
            o.cross = Some(input(&root, "exit2.json", &j));
            let c = &run(&o).unwrap().cells[1].0;
            assert_eq!(c.status, want, "{text}: {c:?}");
            assert!(c.evidence.contains(text), "{text}: {}", c.evidence);
        }
        // A later commit that changes Cargo.lock makes the reading stale.
        o.host = HOST.into();
        o.cross = Some(input(
            &root,
            "exit2.json",
            &exit2_json(&head, ["pass", "pass"], true),
        ));
        write(&root, "Cargo.lock", "# lock 2\n");
        commit_all(&root, "lock");
        let c = &run(&o).unwrap().cells[1].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(c.evidence.contains("Cargo.lock"), "{}", c.evidence);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn gc_json(fails: &[&str], extra: &str) -> String {
        let total = t262_files().len() as u64 - 1;
        format!(
            r#"{{"command":"exec","target":"{HOST}","gc_stress":1,"timeout_ms":60000,"feedback_threshold":0,{extra}"skipped":1,"skipped_paths":["test/language/l19.js"],"files":{{"passed":{},"total":{total}}},"results":[{}],"expectations":{{"file":"x"}}}}"#,
            total - fails.len() as u64,
            results(fails)
        )
    }

    const S1: &str = r#""generational":true,"verify_cadence":1,"#;

    #[test]
    fn item4_same_pass_set_with_s1() {
        let root = fake_repo("item4");
        let mut o = opts(&root);
        o.exec = Some(input(
            &root,
            "exec.json",
            &exec_json(
                18,
                20,
                24,
                0.0,
                &["test/language/l00.js", "test/language/l01.js"],
            ),
        ));
        for (j, want, text) in [
            (
                gc_json(&["test/language/l00.js"], S1),
                Status::Pass,
                "S1 on, verify cadence 1, files 22 / 23 + 1 skipped, infra 0, new failures 0",
            ),
            (
                gc_json(&["test/language/l05.js"], S1),
                Status::Fail,
                "pass without GC-stress, fail under it: test/language/l05.js",
            ),
            (gc_json(&[], ""), Status::Open, "no \"generational\" field"),
            (
                gc_json(&[], r#""generational":true,"verify_cadence":8,"#),
                Status::Fail,
                "verify cadence 8, not 1",
            ),
            (
                gc_json(&[], r#""generational":false,"verify_cadence":1,"#),
                Status::Fail,
                "S1 off",
            ),
            (
                gc_json(&[], S1).replace(r#""timeout_ms":60000"#, r#""timeout_ms":90000"#),
                Status::Fail,
                "timeout_ms 90000",
            ),
            (
                gc_json(&[], S1)
                    .replace(r#"["test/language/l19.js"]"#, r#"["test/language/l18.js"]"#),
                Status::Fail,
                "not in the list: test/language/l18.js",
            ),
        ] {
            o.exec_gc = vec![input(&root, "gc.json", &j)];
            let c = &run(&o).unwrap().cells[3].0;
            assert_eq!(c.status, want, "{text}: {c:?}");
            assert!(c.evidence.contains(text), "{text}: {}", c.evidence);
            assert!(c.evidence.contains("skip list expectations/test262-gc-skip.txt: 1 paths (m0-exit: 1), 0 added since"), "{}", c.evidence);
        }
        // Without --exec: against the committed expectation file (the run must name it).
        o.exec = None;
        // (A Windows temp path has backslashes: escape them for JSON.)
        let named = gc_json(&["test/language/l00.js"], S1).replace(
            r#""file":"x""#,
            &format!(
                r#""file":"{}""#,
                root.join(format!("expectations/test262.{HOST}.jit.txt"))
                    .display()
                    .to_string()
                    .replace('\\', "\\\\")
            ),
        );
        o.exec_gc = vec![input(&root, "gc.json", &named)];
        let c = &run(&o).unwrap().cells[3].0;
        assert_eq!(c.status, Status::Pass, "{c:?}");
        assert!(c.evidence.contains("(no --exec given)"), "{}", c.evidence);
        // V9-A24: a path added to the skip list needs a decisions-log line that names it.
        write(
            &root,
            "expectations/test262-gc-skip.txt",
            "test/language/l19.js\ntest/language/l18.js\n",
        );
        commit_all(&root, "skip");
        let c = &run(&o).unwrap().cells[3].0;
        assert_eq!(c.status, Status::Fail, "{c:?}");
        assert!(c.evidence.contains("2 paths (m0-exit: 1), 1 added since; not named in docs/decisions-log.md: test/language/l18.js"), "{}", c.evidence);
        write(
            &root,
            "docs/decisions-log.md",
            "# log\n- GC-SKIP-1: test/language/l18.js times out.\n",
        );
        let c = &run(&o).unwrap().cells[3].0;
        assert!(
            c.evidence
                .contains("1 added since, each named in docs/decisions-log.md"),
            "{}",
            c.evidence
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn item4_windows_needs_every_shard_and_s1() {
        let job = |k: u32, step: &str, concl: &str| {
            format!(
                r#"{{"name":"test262 GC-stress (windows-x64, {k}/2)","steps":[{{"name":"{step}","status":"completed","conclusion":"{concl}"}}]}}"#
            )
        };
        let ci = |jobs: &[String]| {
            Ci::from_json(&parse_json(&format!(r#"{{"jobs":[{}]}}"#, jobs.join(","))).unwrap())
                .unwrap()
        };
        let s1 = "test262 exec, GC-stress S1 + eager feedback";
        let s0 = "test262 exec, GC-stress + eager feedback";
        let c = gc_ci_cell(&ci(&[job(1, s1, "success"), job(2, s1, "success")])).unwrap();
        assert_eq!(c.status, Status::Pass, "{c:?}");
        let c = gc_ci_cell(&ci(&[job(1, s0, "success"), job(2, s0, "success")])).unwrap();
        assert_eq!(c.status, Status::Open, "{c:?}");
        let c = gc_ci_cell(&ci(&[job(1, s1, "success")])).unwrap();
        assert_eq!(c.status, Status::Fail, "{c:?}");
        let c = gc_ci_cell(&ci(&[job(1, s1, "success"), job(2, s1, "failure")])).unwrap();
        assert_eq!(c.status, Status::Fail, "{c:?}");
        assert!(gc_ci_cell(&ci(&[])).is_err());
    }

    fn fuzz_json(tool: &str, commit: &str, hours: f64, counted: bool, crash: &str) -> String {
        format!(
            r#"{{"schema":"ep9-fuzz/v1","tool":"{tool}","target":"t","commit":"{commit}","cpu_hours":{hours},"counted":{counted},"crashes":[{crash}]}}"#
        )
    }

    #[test]
    fn item5_counts_cpu_hours_and_open_crashes() {
        let root = fake_repo("item5");
        let head = git_in(&root, &["rev-parse", "HEAD"]);
        let mut o = opts(&root);
        let fixed = r#"{"id":"a1","state":"fixed"}"#;
        let set = |o: &mut Options, a: String, b: String| {
            o.fuzz = vec![input(&root, "f1.json", &a), input(&root, "f2.json", &b)];
        };
        set(
            &mut o,
            fuzz_json("fuzzilli", &head, 100.0, true, fixed),
            fuzz_json("cargo-fuzz", &head, 24.0, true, ""),
        );
        let (h, w) = &run(&o).unwrap().cells[4];
        assert_eq!(h.status, Status::Pass, "{h:?}");
        assert_eq!(w.status, Status::Pass);
        assert!(
            h.evidence.contains(
                "Fuzzilli 100.0 / 100 CPU-h, cargo-fuzz 24.0 / 24 CPU-h, 0 open unique crashes"
            ),
            "{}",
            h.evidence
        );
        set(
            &mut o,
            fuzz_json(
                "fuzzilli",
                &head,
                100.0,
                true,
                r#"{"id":"b2","state":"open"}"#,
            ),
            fuzz_json("cargo-fuzz", &head, 24.0, true, ""),
        );
        assert_eq!(run(&o).unwrap().cells[4].0.status, Status::Fail);
        set(
            &mut o,
            fuzz_json("fuzzilli", &head, 99.0, true, ""),
            fuzz_json("cargo-fuzz", &head, 24.0, true, ""),
        );
        let c = &run(&o).unwrap().cells[4].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence.contains("fuzzilli: 99.0 counted CPU-h of 100"),
            "{}",
            c.evidence
        );
        // A counted Fuzzilli run before a crates change does not count (V9-A30).
        write(&root, "crates/ep9_vm/src/lib.rs", "// changed\n");
        commit_all(&root, "vm");
        set(
            &mut o,
            fuzz_json("fuzzilli", &head, 100.0, true, ""),
            fuzz_json("cargo-fuzz", &head, 24.0, true, ""),
        );
        let c = &run(&o).unwrap().cells[4].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence.contains(
                "not counted: not this tree: crates, Cargo.lock, rust-toolchain.toml changed"
            ),
            "{}",
            c.evidence
        );
        assert!(
            c.evidence.contains("cargo-fuzz 24.0 / 24"),
            "{}",
            c.evidence
        );
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
        let o = a(&[
            "--host",
            "x",
            "--out",
            "o",
            "--exec-gc",
            "a",
            "--exec-gc",
            "b",
            "--fuzz",
            "f",
            "--fuzz",
            "g",
            "--verdicts",
            "v",
            "--kernels",
            "k",
            "--cross",
            "c",
            "--exec",
            "e",
            "--nesting",
            "n",
            "--ci-run",
            "12",
            "--ci-run-json",
            "j",
            "--root",
            "r",
        ])
        .unwrap();
        assert_eq!((o.exec_gc.len(), o.fuzz.len(), o.verdicts.len()), (2, 2, 1));
        assert_eq!(o.base, "m0-exit");
        assert_eq!(o.root, PathBuf::from("r"));
        assert_eq!(
            a(&["--host", "x", "--out", "o", "--base", "d7cc797"])
                .unwrap()
                .base,
            "d7cc797"
        );
        assert!(a(&["--host", "x"]).is_err());
        assert!(a(&["--host", "x", "--out", "o", "--ci-run", "12a"]).is_err());
        assert!(a(&["--host", "x", "--out", "o", "--what"]).is_err());
        assert!(a(&["--host", "x", "--out", "o", "--base", ""]).is_err());
        assert!(a(&["--host"]).is_err());
    }
}
