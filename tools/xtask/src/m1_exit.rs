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
//! | 1 | kernels JSON of this `--host`: 15 of 15 kernels "runs, output equal", each of the 15 kernel ids once (a duplicate, missing or unknown id is OPEN), on a fresh tree: a full commit id (40 hex digits) that is a commit here, whose ids of `crates`, `tools/bench`, the root `Cargo.toml`, `Cargo.lock` and `rust-toolchain.toml` are the reported tree ids, no uncommitted change, the reported tree ids equal HEAD's, and its ids of `third_party/regress.commit` and `third_party/patches/regress` (the regress fork) equal HEAD's (else OPEN) | job `bench-run (windows-x64)`, step `kernels` (month 5; NOT-RUN without it) |
//! | 2 | exit2 JSON: both readings `pass` (a `fail` is FAIL; marginal or no gate verdict is OPEN), each of the two reading ids once (else OPEN); a gate host, not provisional; fresh: its commit is a full commit id (40 hex digits) and is HEAD, or an ancestor with no change in the five paths of item 1 and the two regress fork paths since, and `"dirty": false` (run.mjs writes it, exit2.mjs copies it; `true` or no field is OPEN). On `x86_64-unknown-linux-gnu` without a gate host the cell reads OPEN "marginal (no gate host)" (V9-D31) | N/A by design |
//! | 3 | exec JSON: no GC-stress, 0 infra, `language/` >= 85% of the whole `test/language` dir of this tree's test262 checkout; a fresh, named input (below) | step `test262 exec` green; passed = total (host exec JSON) - the `test/language/` lines of the Windows expectation file |
//! | 4 | GC JSONs (one run, or every shard of one split, each once): `gc_stress` > 0, `timeout_ms` <= 60000, `feedback_threshold` 0, S1 on (`"generational": true`), verify cadence 1 (`"verify_cadence": 1`; the row prints it), 0 infra, the whole corpus, the skipped tests = the committed skip list; the same pass set: with `--exec`, no test that passes there fails under GC-stress; without it, 0 new failures against the committed expectation file. The row prints the skip-list size and FAILs if it grew since `--base` without a `docs/decisions-log.md` line that names each added path (V9-A19, V9-A24); each GC JSON a fresh, named input (below), all of one commit, the `--exec` JSON of that commit too | every job `test262 GC-stress (windows-x64, k/n)` once, its step `test262 exec, GC-stress ...` green, and the step name says `S1` (else OPEN) |
//! | 5 | fuzz records: Fuzzilli >= 100 CPU-h and cargo-fuzz >= 24 CPU-h (counted records, each campaign once: a file given twice or a second record of a `campaign` id that already counted adds nothing, while a record of the id that is not counted does not block a later one (F4); a counted record with no `campaign` id is not counted, OPEN), 0 open unique crashes; a counted record names a full commit id (40 hex digits), says `"dirty": false` (`true`, no field or a non-bool: not counted, OPEN), and a counted Fuzzilli record is fresh in `crates`, the root `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `third_party/regress.commit`, `third_party/patches/regress` (V9-A30, D41), a counted cargo-fuzz record in `crates/ep9_frontend`, `crates/ep9_bytecode`, `fuzz`, the root `Cargo.toml`, `Cargo.lock` (else OPEN) | repo-wide, as the host |
//! | 6 | `adr-lint --base <rev>` passes (all five parts; part 5, the doc 13 short cites, gates from M4-P5) | repo-wide, as the host |
//! | 7 | the Spike C verdict is GO (no condition); H2, H14, H16, H17, H18, H19 Frozen, part by part (`exit::freeze`) | repo-wide, as the host |
//! | 8 | nesting log: the 7 `depth_tests::m1_*` tests, `caps_hold_end_to_end` and `every_m1_form_is_listed` ok, each in a cargo run that exited 0, no cargo failure line, every run with its start and exit-status lines; a fresh, named input: its `m1-exit source:` line (below) | step `cargo test --workspace --locked` of `build + test (windows-x64)` green |
//! | 9 | H29-H34 Frozen, each part of its Freeze table that freezes by M1 exit (an append-only list is open, a later gate is not due; MC-w14-15) | repo-wide, as the host |
//! | 10 | the SP-Facts verdict is GO, or "GO, speed recorded" (09-milestones.md:241) | repo-wide, as the host |
//!
//! **Fresh, named inputs** (plan v9 section 7 step 3; P3-ORCH-14 finding 6, P3-ORCH-21): the
//! exec and GC JSONs (`commit`, `dirty`, `made_by`, written by tools/test262/src/provenance.rs)
//! and the nesting log (`m1-exit source: commit <sha> dirty <n> made-by <host or job>`, written
//! by NESTING_CMD) name their commit, whether the tree had uncommitted changes, and the host or
//! CI job that made them. The cell is at least OPEN, with a "stale input" or "provenance" reason,
//! when the commit is missing, is not HEAD or an ancestor with no change in `crates`,
//! `tools/bench`, the root `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `tools/test262`,
//! `third_party/test262.commit`, `third_party/regress.commit` or `third_party/patches/regress`
//! since (unless the input names the git id of each of these paths
//! and each equals HEAD's: a CI pull_request job runs on GitHub's merge commit, which the local
//! repository does not have; the JSON `"tree"` object, the log line `m1-exit trees: <path>=<id>
//! ...`), when the tree was dirty (or the input does not say), when the runner read another
//! source state at the end of the run than at its start (`"source_changed_during_run": true`),
//! when no host or job is named, when the runner read its source through its test seam
//! (`"test_seam": true`, F3), or when the job is of another CI run than `--ci-run`. An
//! ep9-test262 JSON also names the commit its runner was built at (`"runner_commit"`), whether
//! the runner's compiled sources had uncommitted changes then (`"runner_dirty"`), and the HEAD of
//! the test262 checkout it ran before and after the run (`"test262_head"`, `"test262_head_end"`)
//! and whether that checkout had changes (`"test262_dirty"`); the cell is OPEN ("provenance")
//! when one is missing or not a full commit id, the runner commit is not the input's `commit`,
//! the runner was built dirty, the checkout HEAD is not the pin `third_party/test262.commit` or
//! moved during the run, or the checkout had changes (plan v10 X4).
//!
//! **The gate build** (X3, the O8 refusal; plan v10 V10-D10, V10-A12): items 1-4 read the
//! build each input ran. An ep9-test262 JSON names `preset`, `mode` (the preset its isolates
//! ran), `feedback` and `features`; the kernels JSON (`ep9.engine_build`) and the exit2 JSON
//! (`engine_build`) copy the build line of `ep9 --version`, `features <list>; preset <name>`,
//! whose feedback is the preset's default (the `ep9` binary has no feedback option). The cell
//! is OPEN (not FAIL), "preset X, the gate reads Y", when a field is missing, the preset or mode
//! is not `jit`, the feedback is not `full`, or the features do not hold `jit`. Item 4 reads
//! the build of each GC-stress JSON and of the `--exec` run it compares with.
//!
//! **The CI run** names its head (`headSha`): HEAD, or an ancestor with no change in those paths,
//! `expectations` or `.github` since. Else every Windows cell is at least OPEN ("stale CI run").
//!
//! **Fuzz records** (`ep9-fuzz/v1`, one JSON object per campaign; plan v9 V9-D24, V9-A30):
//! `{"schema": "ep9-fuzz/v1", "tool": "fuzzilli" | "cargo-fuzz", "campaign": "<id>", "target": "<name>", "commit":
//! "<sha>", "dirty": true | false, "cpu_hours": <n>, "counted": true | false, "crashes": [{"id":
//! "<hash>", "state": "open" | "fixed" | "duplicate" | "not-a-bug"}]}`. A record with `counted:
//! false` only adds its open crashes. `dirty` (the fuzzed tree had uncommitted changes in a
//! tracked file): only `false` is counted; `true`, no field or a non-bool is not counted (OPEN).
//! Its producers (fuzz.yml, infra/fuzz) are month-5 work and do not write it yet, so item 5 stays
//! OPEN until they do (CX6-1, replaces the CX5-C5 NOTE rule). The record holds hashes only, never
//! an input (rule 14).
//!
//! The extra rows (no exit-code effect): the carried M0 item-5 cell (SP-Dispatch, read from the
//! newest `*-m0-spike-review.md`), the stub markers (`m2`, `m3`, `m4`, plan v9 rule 4; `m5`,
//! plan v10 rule 4) and the ADR Counts line. The tree root is `--root`, or this workspace (as
//! m0-exit). The shared readers are in `crate::exit`.
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
use crate::exit::report::{Cell, Status, all_ok, cell, not_given, some_paths, stub_markers};
use crate::exit::t262::{committed, count, exec_health, expectation_set, pct, recheck};
use crate::exit::verdict::{newest_spike_review, verdict_table};
use crate::exit::{git, read};

/// The stub markers of plan v9 rule 4 (V9-D37) and plan v10 rule 4 (`m5`, V10-D54). Written in
/// two parts, so this file does not hold them (m0-exit greps for the first two).
const STUB_MARKERS: [&str; 4] = [
    concat!("m2", "-stub"),
    concat!("m3", "-stub"),
    concat!("m4", "-stub"),
    concat!("m5", "-stub"),
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
/// GATES-3 tool note adds `Cargo.lock` and `rust-toolchain.toml` to the M0 rule; M4-P5 C1 adds
/// the root `Cargo.toml`, whose `[profile.release]` sets `lto` and `panic`). Items 2 and 5 use
/// them too.
const FRESH_PATHS: [&str; 5] = [
    "crates",
    "tools/bench",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
];

/// The regress fork (D41, R41; pre medium 1): its pin and its patch set. ep9_vm builds regress
/// from the git-ignored checkout these two make (scripts/sync-regress.sh, `[patch.crates-io]`),
/// so a change of either changes engine code. Every fresh list of an input whose code builds
/// regress has them: items 1 and 2 ([`ENGINE_FRESH_PATHS`]), 3, 4 and 8, the CI run, a Fuzzilli
/// record (not cargo-fuzz: its targets do not build regress).
const REGRESS_PATHS: [&str; 2] = ["third_party/regress.commit", "third_party/patches/regress"];

/// Item 2: [`FRESH_PATHS`] and [`REGRESS_PATHS`]. Item 1 keeps [`FRESH_PATHS`] (the tree ids
/// kernels.mjs writes) and checks [`REGRESS_PATHS`] at the input's commit against HEAD.
const ENGINE_FRESH_PATHS: [&str; 7] = [
    "crates",
    "tools/bench",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    REGRESS_PATHS[0],
    REGRESS_PATHS[1],
];

/// Items 3, 4 and 8 (plan v9 section 7 step 3, P3-ORCH-14): the paths whose change after the
/// input's commit makes a test262 or nesting input stale: [`FRESH_PATHS`], the runner and the
/// test262 pin.
const T262_FRESH_PATHS: [&str; 9] = [
    "crates",
    "tools/bench",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "tools/test262",
    "third_party/test262.commit",
    REGRESS_PATHS[0],
    REGRESS_PATHS[1],
];

/// Item 5 (CX5-C4, F2): the paths whose change after a counted cargo-fuzz record's commit makes
/// it stale: the `fuzz` workspace, every engine crate it builds (the path dependencies of
/// `fuzz/Cargo.toml` and theirs; a test checks the list against them), the root `Cargo.toml`
/// (its `[profile.release]`, the workspace members) and `Cargo.lock`.
const CARGO_FUZZ_FRESH_PATHS: [&str; 5] = [
    "crates/ep9_frontend",
    "crates/ep9_bytecode",
    "fuzz",
    "Cargo.toml",
    "Cargo.lock",
];

/// The Windows column (P3-ORCH-21): the CI run's head is HEAD, or an ancestor with no change in
/// these paths since ([`T262_FRESH_PATHS`], the expectation files and the workflows).
const CI_FRESH_PATHS: [&str; 11] = [
    "crates",
    "tools/bench",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "tools/test262",
    "third_party/test262.commit",
    REGRESS_PATHS[0],
    REGRESS_PATHS[1],
    "expectations",
    ".github",
];

/// Item 8: the source line of a nesting log (NESTING_CMD): `m1-exit source: commit <sha> dirty
/// <n> made-by <host or job>`, inside a cargo run (after its start line), so m0-exit, which reads
/// the same log, sees one more body line.
const SOURCE_LINE: &str = "m1-exit source: ";

/// Item 8: the trees line of a nesting log (NESTING_CMD): `m1-exit trees: <path>=<id> ...` for
/// [`T262_FRESH_PATHS`], so a log made on GitHub's pull_request merge commit (which the local
/// repository does not have) can show that it ran HEAD's code.
const TREES_LINE: &str = "m1-exit trees:";

/// Item 1: the kernels (8 v8-v7 + 7 Octane).
const KERNELS: usize = 15;
/// Item 1: the kernel ids, each once (CX5-C3; `KERNEL_IDS` of tools/bench/cross/kernels.mjs,
/// whose node test checks that the two lists are the same).
const KERNEL_IDS: [&str; KERNELS] = [
    "v8v7/richards",
    "v8v7/deltablue",
    "v8v7/crypto",
    "v8v7/raytrace",
    "v8v7/earley-boyer",
    "v8v7/regexp",
    "v8v7/splay",
    "v8v7/navier-stokes",
    "octane/richards",
    "octane/deltablue",
    "octane/crypto",
    "octane/raytrace",
    "octane/earley-boyer",
    "octane/splay",
    "octane/navier-stokes",
];
/// Item 2: the reading ids, each once (CX5-C3; `READINGS` of tools/bench/cross/exit2.mjs).
const READING_IDS: [&str; 2] = ["octane7-vs-qjs", "v8v7-7-vs-node-jitless"];
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

/// Items 1-4, X3 (the O8 refusal; plan v10 V10-D10, V10-A12): the gate build. The gate reads
/// the default build: the `jit` ep9_vm feature, preset `jit`, feedback `full`. An input of
/// another build, or one that does not say, is OPEN (not FAIL): `ics_only` and `jitless` are
/// extra columns, never the gate reading.
const GATE_PRESET: &str = "jit";
const GATE_FEEDBACK: &str = "full";
const GATE_FEATURE: &str = "jit";

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
     echo \"m1-exit source: commit $(git rev-parse HEAD) dirty $(git status --porcelain --untracked-files=no | wc -l | tr -d ' ') made-by host:$(hostname)${GITHUB_RUN_ID:+ gh-run:$GITHUB_RUN_ID/$GITHUB_JOB}\" >> nesting.log; \
     echo \"m1-exit trees:$(for p in crates tools/bench Cargo.toml Cargo.lock rust-toolchain.toml tools/test262 third_party/test262.commit third_party/regress.commit third_party/patches/regress; do printf ' %s=%s' \"$p\" \"$(git rev-parse --verify --quiet \"HEAD:$p\")\"; done)\" >> nesting.log; \
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
    /// The CI run's head against HEAD ([`ci_fresh`]); Err(why) makes each Windows cell OPEN.
    ci_fresh: Result<String, String>,
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
    let ci_fresh = match &ci {
        Ok(ci) => ci_fresh(&o.root, ci),
        Err(e) => Err(e.clone()),
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
        ci_fresh,
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

/// The Windows cell from the CI run: NOT-RUN without a run; a missing job or step is NOT-RUN. A
/// run of another tree ([`ci_fresh`]) proves nothing about HEAD: its PASS reads OPEN, with the
/// reason first (P3-ORCH-21).
fn with_ci(c: &Ctx, f: impl FnOnce(&Ci) -> Result<Cell, String>) -> Cell {
    match &c.ci {
        Err(e) => cell(Status::NotRun, e.clone()),
        Ok(ci) => {
            let got = f(ci).unwrap_or_else(|e| cell(Status::NotRun, e));
            match &c.ci_fresh {
                Ok(_) => got,
                Err(why) => cell(
                    got.status.max(Status::Open),
                    format!("stale CI run: {why}; the run read: {}", got.evidence),
                ),
            }
        }
    }
}

/// The CI run's head against HEAD (P3-ORCH-21): the run names its head commit, and that is HEAD
/// or an ancestor with no change in [`CI_FRESH_PATHS`] since. Err(why) gives OPEN.
fn ci_fresh(root: &Path, ci: &Ci) -> Result<String, String> {
    let id = ci.id.as_deref().unwrap_or("(no id)");
    let Some(head) = ci.head.as_deref().filter(|h| !h.is_empty()) else {
        return Err(format!("CI run {id} names no head commit (headSha)"));
    };
    if !is_full_sha(head) {
        return Err(format!(
            "CI run {id}: its headSha {head:?} is not a commit id (40 hex digits)"
        ));
    }
    fresh_since(root, head, &CI_FRESH_PATHS).map_err(|e| format!("CI run {id}: {e}"))
}

/// A full commit id: 40 lower-case hex digits. An input names its commit this way (the runner,
/// NESTING_CMD and `gh run view` all write it); a ref name such as `HEAD` or a branch would
/// resolve to whatever the local tree is now, so it names no commit (verifier r1 F3).
fn is_full_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// [`fresh_since`] for the `commit` of an item 2 or item 5 input: a named commit must be a full
/// commit id ([`is_full_sha`]), so `"HEAD"` or a short id gives OPEN (M4-P5 C3). A missing one
/// keeps the [`fresh_since`] text.
fn fresh_named(root: &Path, commit: &str, paths: &[&str]) -> Result<String, String> {
    if !commit.is_empty() && commit != "unknown" && !is_full_sha(commit) {
        return Err(format!(
            "its commit {commit:?} is not a commit id (40 hex digits)"
        ));
    }
    fresh_since(root, commit, paths)
}

/// Where an input came from (P3-ORCH-14, P3-ORCH-21): the `commit`, `dirty` and `made_by`
/// fields of an ep9-test262 JSON (tools/test262/src/provenance.rs), or the [`SOURCE_LINE`] of a
/// nesting log.
#[derive(Debug, Clone, Default, PartialEq)]
struct Source {
    commit: Option<String>,
    dirty: Option<bool>,
    made_by: Option<String>,
    /// (path, id) of the paths the input names (`"tree"` of the JSON, the [`TREES_LINE`] of a log).
    trees: BTreeMap<String, String>,
    /// The runner read another commit, dirty flag or tree ids at the end of the run than at its
    /// start (`"source_changed_during_run": true`, CX5-C1).
    changed: bool,
    /// An ep9-test262 JSON (not a log): it must also name the runner's build commit and the
    /// test262 checkout HEAD (X4).
    runner: Option<Runner>,
    /// An ep9-test262 JSON (not a log): the build it ran, which must be the gate build (X3).
    build: Option<Build>,
    /// The runner read its source facts through its test seam (`"test_seam": true`; F3,
    /// tools/test262/src/exec.rs): another tree's facts, or a file changed on purpose.
    seam: bool,
}

/// The build an input names (X3, [`build_check`]): an ep9-test262 JSON writes `preset` (the
/// build's default preset), `mode` (the preset its isolates ran), `feedback` and `features`
/// (tools/test262/src/exec.rs, MC-w17-5); a bench JSON copies the second line of
/// `ep9 --version`, `features <list>; preset <name>` ([`Build::of_line`]). A field that is
/// missing or not of its type is None.
#[derive(Debug, Clone, Default, PartialEq)]
struct Build {
    preset: Option<String>,
    /// The preset the isolates ran (`mode`); None when the input has no such field.
    mode: Option<String>,
    feedback: Option<String>,
    /// True when `feedback` is not written but follows from the preset ([`Build::of_line`]).
    feedback_implied: bool,
    features: Option<Vec<String>>,
}

impl Build {
    /// The build fields of an ep9-test262 JSON.
    fn of_json(j: &Json) -> Build {
        Build {
            preset: str_field(j, "preset"),
            mode: str_field(j, "mode"),
            feedback: str_field(j, "feedback"),
            feedback_implied: false,
            features: j.get("features").and_then(Json::arr).and_then(|a| {
                a.iter()
                    .map(|f| f.str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()
            }),
        }
    }

    /// The build line of `ep9 --version` (`features jit,become; preset jit`; `features none`
    /// when no feature is on; crates/ep9_cli/src/args.rs `version_text`), as kernels.mjs
    /// (`ep9.engine_build`) and run.mjs copy it; None (no line, null) gives a build with no
    /// field. A `feedback <mode>` part is read when there is one; else the feedback follows
    /// from the preset (`jit` full, `jitless` ics_only; crates/ep9_vm/src/exec.rs
    /// `ExecOptions::jit`, `::jitless`): the `ep9` binary has no feedback option and runs
    /// `ExecOptions::default()`, the build's preset with its default mode.
    fn of_line(line: Option<&str>) -> Build {
        let mut b = Build::default();
        for part in line.unwrap_or("").split(';') {
            match part.trim().split_once(' ') {
                Some(("features", v)) => {
                    b.features = Some(match v.trim() {
                        "none" => Vec::new(),
                        v => v.split(',').map(|f| f.trim().to_string()).collect(),
                    });
                }
                Some(("preset", v)) => b.preset = Some(v.trim().to_string()),
                Some(("feedback", v)) => b.feedback = Some(v.trim().to_string()),
                _ => {}
            }
        }
        if b.feedback.is_none() {
            b.feedback = match b.preset.as_deref() {
                Some("jit") => Some("full".to_string()),
                Some("jitless") => Some("ics_only".to_string()),
                _ => None,
            };
            b.feedback_implied = b.feedback.is_some();
        }
        b
    }

    /// `preset jit, feedback full, features jit,become` (`?` for a missing field).
    fn text(&self) -> String {
        format!(
            "build: preset {}, feedback {}{}, features {}",
            self.preset.as_deref().unwrap_or("?"),
            self.feedback.as_deref().unwrap_or("?"),
            if self.feedback_implied {
                " (the preset's default)"
            } else {
                ""
            },
            match &self.features {
                None => "?".to_string(),
                Some(f) if f.is_empty() => "none".to_string(),
                Some(f) => f.join(","),
            }
        )
    }
}

/// X3, the O8 refusal (plan v10 section 3.7, V10-D10): the OPEN reasons of an input whose build
/// is not the gate build ([`GATE_PRESET`], [`GATE_FEEDBACK`], a feature list with
/// [`GATE_FEATURE`]), or that does not name it. Each reason reads "preset X, the gate reads Y".
fn build_check(b: &Build) -> Vec<String> {
    let mut why = Vec::new();
    let mut field = |what: &str, key: &str, got: Option<&str>, want: &str| match got {
        None => why.push(format!(
            "build: it names no {what} (`{key}`), the gate reads {what} {want} (V10-D10)"
        )),
        Some(g) if g != want => why.push(format!(
            "build: {what} {g}, the gate reads {what} {want} (V10-D10)"
        )),
        Some(_) => {}
    };
    field("preset", "preset", b.preset.as_deref(), GATE_PRESET);
    field("feedback", "feedback", b.feedback.as_deref(), GATE_FEEDBACK);
    // `mode` is the preset the isolates ran; an input without the field is read by `preset`.
    if let Some(m) = b.mode.as_deref()
        && m != GATE_PRESET
    {
        why.push(format!(
            "build: its isolates ran preset {m} (`mode`), the gate reads preset {GATE_PRESET} (V10-D10)"
        ));
    }
    match &b.features {
        None => why.push(format!(
            "build: it names no features (`features`), the gate reads a build with {GATE_FEATURE} (V10-D10)"
        )),
        Some(f) if !f.iter().any(|f| f == GATE_FEATURE) => why.push(format!(
            "build: features {}, the gate reads a build with {GATE_FEATURE} (V10-D10)",
            if f.is_empty() {
                "none".to_string()
            } else {
                f.join(",")
            }
        )),
        Some(_) => {}
    }
    why
}

/// The build facts an ep9-test262 JSON names from M5-P0 (plan v10 section 2.6, X4): the
/// commit the runner binary was built at (`"runner_commit"`, C2) and the HEAD of the test262
/// checkout it ran (`"test262_head"`, item 8). A field that is missing or empty is None.
#[derive(Debug, Clone, Default, PartialEq)]
struct Runner {
    commit: Option<String>,
    /// `"runner_dirty"` (gates5 high 1): the compiled sources had uncommitted changes when the
    /// runner was built.
    dirty: Option<bool>,
    test262_head: Option<String>,
    /// `"test262_head_end"` and `"test262_dirty"` (gates5 high 2): the checkout HEAD read
    /// again at the end of the run, and whether the checkout had changes before or after it.
    test262_head_end: Option<String>,
    test262_dirty: Option<bool>,
}

impl Source {
    fn of_json(j: &Json) -> Source {
        Source {
            commit: j
                .get("commit")
                .and_then(Json::str)
                .filter(|c| !c.is_empty())
                .map(str::to_string),
            dirty: j.get("dirty").and_then(Json::bool),
            made_by: j
                .get("made_by")
                .and_then(Json::str)
                .filter(|m| !m.trim().is_empty())
                .map(str::to_string),
            trees: match j.get("tree") {
                Some(Json::Obj(m)) => m
                    .iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.str()?.to_string())))
                    .collect(),
                _ => BTreeMap::new(),
            },
            changed: j.get("source_changed_during_run").and_then(Json::bool) == Some(true),
            runner: Some(Runner {
                commit: str_field(j, "runner_commit"),
                dirty: j.get("runner_dirty").and_then(Json::bool),
                test262_head: str_field(j, "test262_head"),
                test262_head_end: str_field(j, "test262_head_end"),
                test262_dirty: j.get("test262_dirty").and_then(Json::bool),
            }),
            build: Some(Build::of_json(j)),
            seam: j.get("test_seam").and_then(Json::bool) == Some(true),
        }
    }

    /// The source lines of a log: none gives an empty source (no commit); lines that differ, or
    /// one in another form, are Err(why).
    fn of_log(log: &str) -> Result<Source, String> {
        let mut found: Option<Source> = None;
        let mut trees: Option<BTreeMap<String, String>> = None;
        for l in log.lines().map(str::trim) {
            if let Some(rest) = l.strip_prefix(TREES_LINE) {
                let mut t = BTreeMap::new();
                for pair in rest.split_whitespace() {
                    // `<path>=` (empty id): HEAD has no such path.
                    let (p, id) = pair
                        .split_once('=')
                        .filter(|(p, _)| !p.is_empty())
                        .ok_or_else(|| format!("a trees line in another form: {l:?}"))?;
                    if !id.is_empty() {
                        t.insert(p.to_string(), id.to_string());
                    }
                }
                if trees.as_ref().is_some_and(|old| *old != t) {
                    return Err("two trees lines that differ".into());
                }
                trees = Some(t);
                continue;
            }
            let Some(rest) = l.strip_prefix(SOURCE_LINE) else {
                continue;
            };
            let bad = || format!("a source line in another form: {l:?}");
            let rest = rest.strip_prefix("commit ").ok_or_else(bad)?;
            let (commit, rest) = rest.split_once(" dirty ").ok_or_else(bad)?;
            let (dirty, made_by) = rest.split_once(" made-by ").ok_or_else(bad)?;
            let dirty: u64 = dirty.trim().parse().map_err(|_| bad())?;
            let s = Source {
                commit: Some(commit.trim().to_string()).filter(|c| !c.is_empty()),
                dirty: Some(dirty > 0),
                made_by: Some(made_by.trim().to_string()).filter(|m| !m.is_empty()),
                trees: BTreeMap::new(),
                changed: false,
                runner: None,
                build: None,
                seam: false,
            };
            match &found {
                Some(f) if *f != s => {
                    return Err(format!(
                        "two source lines that differ: commit `{}` and `{}`",
                        short(f.commit.as_deref().unwrap_or("none")),
                        short(s.commit.as_deref().unwrap_or("none"))
                    ));
                }
                _ => found = Some(s),
            }
        }
        let mut s = found.unwrap_or_default();
        s.trees = trees.unwrap_or_default();
        Ok(s)
    }

    /// The CI run id of `made_by` (` gh-run:<id>/<job>`).
    fn ci_run(&self) -> Option<&str> {
        let m = self.made_by.as_deref()?;
        let rest = &m[m.find("gh-run:")? + "gh-run:".len()..];
        Some(rest.split(['/', ' ']).next().unwrap_or(rest)).filter(|r| !r.is_empty())
    }
}

/// A non-empty string field of a JSON object, trimmed.
fn str_field(j: &Json, key: &str) -> Option<String> {
    j.get(key)
        .and_then(Json::str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// The test262 pin of this tree: `third_party/test262.commit` at HEAD, or the file itself
/// outside a git tree; trimmed, lower case.
fn test262_pin(root: &Path) -> Option<String> {
    git(root, &["show", "HEAD:third_party/test262.commit"])
        .or_else(|| std::fs::read_to_string(root.join("third_party/test262.commit")).ok())
        .map(|p| p.trim().to_ascii_lowercase())
        .filter(|p| !p.is_empty())
}

/// X4 (plan v10 section 2.6; C2 and in:193 item 8): the OPEN reasons of an ep9-test262 JSON's
/// build facts. The runner was built at the input's `commit` (a runner of another commit may
/// run other code than the tree it names) with no uncommitted change in its compiled sources
/// (gates5 high 1), and the test262 checkout it ran is at this tree's pin, at the same HEAD at
/// the end of the run and with no change (gates5 high 2). A missing fact is OPEN too: the
/// runner of this tree writes each one.
fn runner_check(root: &Path, commit: Option<&str>, r: &Runner) -> Vec<String> {
    let mut why = Vec::new();
    match (r.commit.as_deref(), commit) {
        (None, _) => why.push(
            "provenance: it names no runner build commit (runner_commit; make it again with this tree's runner)"
                .to_string(),
        ),
        (Some(rc), _) if !is_hex_sha(rc) => why.push(format!(
            "provenance: its runner_commit {rc:?} is not a commit id (40 hex digits)"
        )),
        (Some(rc), Some(c)) if !rc.eq_ignore_ascii_case(c) => why.push(format!(
            "provenance: the runner was built at `{}`, the input names `{}`",
            short(rc),
            short(c)
        )),
        _ => {}
    }
    match r.dirty {
        Some(false) => {}
        Some(true) => why.push(
            "provenance: the runner was built from uncommitted changes (runner_dirty)".to_string(),
        ),
        None => why.push(
            "provenance: it names no runner dirty flag (runner_dirty; make it again with this tree's runner)"
                .to_string(),
        ),
    }
    match (r.test262_head.as_deref(), test262_pin(root)) {
        (None, _) => why.push(
            "provenance: it names no test262 checkout HEAD (test262_head; make it again with this tree's runner)"
                .to_string(),
        ),
        (Some(h), _) if !is_hex_sha(h) => why.push(format!(
            "provenance: its test262_head {h:?} is not a commit id (40 hex digits)"
        )),
        (Some(_), None) => {
            why.push("provenance: this tree has no third_party/test262.commit pin".to_string())
        }
        (Some(h), Some(pin)) if !h.eq_ignore_ascii_case(&pin) => why.push(format!(
            "provenance: the test262 checkout was at `{}`, the pin (third_party/test262.commit) is `{}`",
            short(h),
            short(&pin)
        )),
        _ => {}
    }
    // gates5 high 2: the same HEAD at the end of the run, and no change in the checkout.
    match (r.test262_head.as_deref(), r.test262_head_end.as_deref()) {
        (_, None) => why.push(
            "provenance: it names no test262 checkout HEAD at the end of the run (test262_head_end; make it again with this tree's runner)"
                .to_string(),
        ),
        (Some(h), Some(e)) if !h.eq_ignore_ascii_case(e) => why.push(format!(
            "provenance: the test262 checkout moved during the run, from `{}` to `{}`",
            short(h),
            short(e)
        )),
        _ => {}
    }
    match r.test262_dirty {
        Some(false) => {}
        Some(true) => why.push(
            "provenance: the test262 checkout had changes (test262_dirty)"
                .to_string(),
        ),
        None => why.push(
            "provenance: it names no test262 dirty flag (test262_dirty; make it again with this tree's runner)"
                .to_string(),
        ),
    }
    why
}

/// 40 hex digits in either case (gates5 medium 5): a build fact of X4 ([`runner_check`]) must be
/// a full commit id; the runner writes lower case, and upper case names the same id.
fn is_hex_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// [`crate::exit::report::short`] cut on character boundaries (gates5 medium 5): an input field
/// that is no commit id may hold any text, and a cut inside a UTF-8 character would panic. More
/// than 12 characters: the first 8, `…`, the last 5.
fn short(s: &str) -> String {
    let n = s.chars().count();
    if n > 12 {
        let head: String = s.chars().take(8).collect();
        let tail: String = s.chars().skip(n - 5).collect();
        format!("{head}\u{2026}{tail}")
    } else {
        s.to_string()
    }
}

/// The freshness and provenance of one input (plan v9 section 7 step 3; P3-ORCH-14 finding 6,
/// P3-ORCH-21): (the OPEN reasons, a short text). The input names its commit, that commit is
/// HEAD or an ancestor with no change in `paths` since ([`fresh_since`]), the tree had no
/// uncommitted change, and the input names the host or job that made it; a job of another CI
/// run than the one given is a mismatch. An ep9-test262 JSON also names a runner built at that
/// commit and a test262 checkout at the pin ([`runner_check`], X4), and the gate build
/// ([`build_check`], X3).
fn source_check(c: &Ctx, s: &Source, paths: &[&str]) -> (Vec<String>, String) {
    let mut why = Vec::new();
    let tree = match s.commit.as_deref() {
        None => {
            why.push(
                "stale input: it names no commit (make it again with this tree's runner or command)"
                    .to_string(),
            );
            "no commit".to_string()
        }
        Some(commit) if !is_full_sha(commit) => {
            why.push(format!(
                "stale input: its commit {commit:?} is not a commit id (40 hex digits)"
            ));
            format!("commit {commit:?}")
        }
        Some(commit) => match fresh_since(&c.o.root, commit, paths) {
            Ok(t) => format!("commit `{}`: {t}", short(commit)),
            // Another commit (a pull_request merge commit of CI) that ran HEAD's code: every
            // path's id equals HEAD's.
            Err(_) if same_trees(&c.o.root, &s.trees, paths) => format!(
                "commit `{}`, not HEAD or its ancestor, but its ids of {} equal HEAD's",
                short(commit),
                paths.join(", ")
            ),
            Err(e) => {
                why.push(format!("stale input: {e}"));
                format!("commit `{}`", short(commit))
            }
        },
    };
    match s.dirty {
        Some(false) => {}
        Some(true) => why.push("stale input: measured with uncommitted changes (dirty)".into()),
        None => why.push("stale input: it does not say whether the tree was dirty".into()),
    }
    if s.changed {
        why.push(
            "stale input: the source changed during the run (source_changed_during_run)".into(),
        );
    }
    if s.seam {
        why.push(
            "provenance: the runner read its source through its test seam (test_seam): a test run, not a measurement"
                .into(),
        );
    }
    if let Some(r) = &s.runner {
        why.extend(runner_check(&c.o.root, s.commit.as_deref(), r));
    }
    // X3: the gate build (V10-D10).
    if let Some(b) = &s.build {
        why.extend(build_check(b));
    }
    let mut by = match s.made_by.as_deref() {
        Some(m) => m.to_string(),
        None => {
            why.push("provenance: it names no host or job (made_by)".into());
            "no host".to_string()
        }
    };
    if let Some(b) = &s.build {
        let _ = write!(by, "; {}", b.text());
    }
    if let (Some(theirs), Ok(ci)) = (s.ci_run(), &c.ci)
        && let Some(ours) = ci.id.as_deref()
        && theirs != ours
    {
        why.push(format!(
            "provenance: made by CI run {theirs}, but the CI run given is {ours}"
        ));
    }
    (why, format!("{tree}; made by {by}"))
}

/// Every path's id in `trees` is the id of that path at HEAD (a path HEAD does not have must be
/// absent from `trees` too). An empty `trees` is not the same.
fn same_trees(root: &Path, trees: &BTreeMap<String, String>, paths: &[&str]) -> bool {
    !trees.is_empty()
        && paths.iter().all(|p| {
            let ours = git(
                root,
                &["rev-parse", "--verify", "--quiet", &format!("HEAD:{p}")],
            );
            trees.get(*p) == ours.as_ref()
        })
}

/// Adds the source text of an input to a cell; a reason makes the cell at least OPEN.
fn with_source(host: Cell, (why, text): (Vec<String>, String)) -> Cell {
    if why.is_empty() {
        cell(host.status, format!("{}; source: {text}", host.evidence))
    } else {
        cell(
            host.status.max(Status::Open),
            format!("{}; source: {text}. {}", host.evidence, why.join("; ")),
        )
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

/// CX5-C3: the ids of the rows against the `expected` ids, each once: the duplicates
/// (`<what> <id> <n> times`), the missing ids and the unknown ones. Empty when each expected id
/// is there once and no other id is.
fn each_once(what: &str, ids: &[&str], expected: &[&str]) -> Vec<String> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for id in ids {
        *seen.entry(id).or_default() += 1;
    }
    let mut why: Vec<String> = seen
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(id, n)| format!("{what} {id} {n} times"))
        .collect();
    let missing: Vec<&str> = expected
        .iter()
        .filter(|e| !seen.contains_key(*e))
        .copied()
        .collect();
    if !missing.is_empty() {
        why.push(format!("missing {what}s: {}", missing.join(", ")));
    }
    let unknown: Vec<&str> = seen
        .keys()
        .filter(|id| !expected.contains(id))
        .copied()
        .collect();
    if !unknown.is_empty() {
        why.push(format!("unknown {what}s: {}", unknown.join(", ")));
    }
    why
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
    // Each of the 15 kernels once: a copy of a passing row is no evidence for a missing one.
    let ids: Vec<&str> = rows
        .iter()
        .map(|r| r.get("id").and_then(Json::str).unwrap_or("?"))
        .collect();
    let id_why = each_once("kernel", &ids, &KERNEL_IDS);
    if !id_why.is_empty() {
        status = status.max(Status::Open);
        why.extend(id_why);
    }
    if !failing.is_empty() {
        status = Status::Fail;
        why.push(format!("not ok: {}", some_paths(&failing)));
    }
    // Fresh: no uncommitted change, and the tree ids at HEAD equal the measured ones.
    let commit = j.get("commit").and_then(Json::str).unwrap_or("unknown");
    let mut fresh = Vec::new();
    // The tree ids prove the code; the commit must still be a full id (M4-P5 C3).
    if j.get("commit").is_none() {
        fresh.push("it names no ep9 commit".to_string());
    } else if !is_full_sha(commit) {
        fresh.push(format!(
            "its commit {commit:?} is not a commit id (40 hex digits)"
        ));
    } else if git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{commit}^{{commit}}"),
        ],
    )
    .is_none()
    {
        fresh.push(format!(
            "its commit `{}` is not in this repository",
            short(commit)
        ));
    } else {
        // CX5-C2: `kernels.mjs --commit A` names A but reads the tree ids of the checkout: the
        // named commit's ids of the fresh paths must be the reported ones.
        let other: Vec<&str> = FRESH_PATHS
            .iter()
            .filter(|path| {
                let theirs = j.path(&["tree", path]).and_then(Json::str);
                let at = git(
                    root,
                    &[
                        "rev-parse",
                        "--verify",
                        "--quiet",
                        &format!("{commit}:{path}"),
                    ],
                );
                theirs != at.as_deref()
            })
            .copied()
            .collect();
        if !other.is_empty() {
            fresh.push(format!(
                "its commit `{}` has other ids of {} than the reported ones",
                short(commit),
                other.join(", ")
            ));
        }
        // pre medium 1: kernels.mjs writes no ids of the regress fork; its commit's ids must be
        // HEAD's.
        let id = |rev: &str, path: &str| {
            git(
                root,
                &["rev-parse", "--verify", "--quiet", &format!("{rev}:{path}")],
            )
        };
        let moved: Vec<&str> = REGRESS_PATHS
            .iter()
            .filter(|p| id(commit, p) != id("HEAD", p))
            .copied()
            .collect();
        if !moved.is_empty() {
            fresh.push(format!(
                "{} changed between its commit `{}` and HEAD",
                moved.join(", "),
                short(commit)
            ));
        }
    }
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
    // X3: the checked binary is the gate build (its `--version` build line, `ep9.engine_build`).
    let build = Build::of_line(j.path(&["ep9", "engine_build"]).and_then(Json::str));
    let build_why = build_check(&build);
    if !build_why.is_empty() {
        status = status.max(Status::Open);
        why.extend(build_why);
    }
    cell(
        status,
        format!(
            "{}: {ok}/{} {KERNEL_OK}; ep9 commit `{}`; {tree}; {}{}",
            p.display(),
            rows.len(),
            short(commit),
            build.text(),
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
    // Each of the two readings once (CX5-C3).
    let ids: Vec<&str> = readings
        .iter()
        .map(|r| r.get("id").and_then(Json::str).unwrap_or("?"))
        .collect();
    let id_why = each_once("reading", &ids, &READING_IDS);
    if !id_why.is_empty() {
        status = status.max(Status::Open);
        parts.extend(id_why);
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
    let mut tree = match fresh_named(root, commit, &ENGINE_FRESH_PATHS) {
        Ok(t) => t,
        Err(e) => {
            status = status.max(Status::Open);
            e
        }
    };
    // CX5-C5: run.mjs writes whether the fresh paths had uncommitted changes, exit2.mjs copies
    // it; true or missing gives OPEN.
    match j.get("dirty").and_then(Json::bool) {
        Some(false) => {}
        Some(true) => {
            status = status.max(Status::Open);
            let paths: Vec<&str> = j
                .get("dirty_paths")
                .and_then(Json::arr)
                .unwrap_or(&[])
                .iter()
                .filter_map(Json::str)
                .collect();
            let _ = write!(
                tree,
                "; measured with uncommitted changes (dirty): {}",
                if paths.is_empty() {
                    "paths not named".to_string()
                } else {
                    paths.join(", ")
                }
            );
        }
        None => {
            status = status.max(Status::Open);
            tree.push_str("; it does not say whether the tree was dirty (`dirty`)");
        }
    }
    // X3: the measured ep9 is the gate build: the build line of its `--version`
    // (`engine_build`, from the ep9 engine of the run.mjs JSON).
    let build = Build::of_line(j.get("engine_build").and_then(Json::str));
    let build_why = build_check(&build);
    if !build_why.is_empty() {
        status = status.max(Status::Open);
    }
    let _ = write!(tree, "; {}{}", build.text(), because(&build_why));
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
    // Fresh and named (P3-ORCH-14, P3-ORCH-21).
    let host = match &c.exec {
        Some((_, j)) => with_source(
            host,
            source_check(c, &Source::of_json(j), &T262_FRESH_PATHS),
        ),
        None => host,
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
    let host = if c.gc.is_empty() {
        host
    } else {
        with_source(host, gc_source(c))
    };
    let win = with_ci(c, gc_ci_cell);
    (host, win)
}

/// The source of the GC-stress inputs (P3-ORCH-14, P3-ORCH-21): each shard fresh and named, all
/// shards of one commit, and the `--exec` run (the pass set they are compared with) of the same
/// commit, a clean tree, the gate build and not the runner's test seam.
fn gc_source(c: &Ctx) -> (Vec<String>, String) {
    let mut why: Vec<String> = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    let mut commits: BTreeSet<Option<String>> = BTreeSet::new();
    for (_, j) in &c.gc {
        let s = Source::of_json(j);
        let (w, t) = source_check(c, &s, &T262_FRESH_PATHS);
        for w in w {
            if !why.contains(&w) {
                why.push(w);
            }
        }
        if !texts.contains(&t) {
            texts.push(t);
        }
        commits.insert(s.commit);
    }
    if commits.len() > 1 {
        why.push(format!(
            "provenance: the {} GC-stress inputs name {} commits ({}): one split measures one tree",
            c.gc.len(),
            commits.len(),
            commits
                .iter()
                .map(|c| c
                    .as_deref()
                    .map_or("none".to_string(), |c| format!("`{}`", short(c))))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    // The --exec run is the other half of the comparison: it names a commit id, the GC-stress
    // commit, and a clean tree (verifier r1 F4).
    if let Some((ep, ej)) = &c.exec {
        let es = Source::of_json(ej);
        match es.commit.as_deref().filter(|e| is_full_sha(e)) {
            None => why.push(format!(
                "provenance: --exec {} names no commit id: the same pass set needs one tree",
                ep.display()
            )),
            Some(e) => {
                if let [Some(gc)] = commits.iter().collect::<Vec<_>>().as_slice()
                    && e != gc.as_str()
                {
                    why.push(format!(
                        "provenance: --exec {} measured `{}`, the GC-stress run `{}`: the same pass set needs one tree",
                        ep.display(),
                        short(e),
                        short(gc)
                    ));
                }
            }
        }
        if es.dirty != Some(false) {
            why.push(format!(
                "provenance: --exec {} was measured with uncommitted changes, or does not say",
                ep.display()
            ));
        }
        // F3: a pass set the runner made through its test seam is no measurement, as for
        // item 3 (M5-P1 review).
        if es.seam {
            why.push(format!(
                "provenance: --exec {}: the runner read its source through its test seam (test_seam): a test run, not a measurement",
                ep.display()
            ));
        }
        // X3: the pass set the GC-stress runs are compared with is of the gate build too.
        for w in es.build.as_ref().map(build_check).unwrap_or_default() {
            why.push(format!("--exec {}: {w}", ep.display()));
        }
    }
    (why, texts.join(" | "))
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
    // Each campaign counts once (P3-ORCH-21): a file given twice, or a second record of a
    // counted campaign id, adds no hours and no crash.
    let mut files: BTreeSet<PathBuf> = BTreeSet::new();
    let mut campaigns: BTreeSet<String> = BTreeSet::new();
    for (p, j) in &c.fuzz {
        let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
        if !files.insert(canon) {
            why.push(format!("{}: given twice, counted once", p.display()));
            continue;
        }
        if j.get("schema").and_then(Json::str) != Some("ep9-fuzz/v1") {
            status = Status::Fail;
            why.push(format!("{}: not an ep9-fuzz/v1 record", p.display()));
            continue;
        }
        let tool = j.get("tool").and_then(Json::str).unwrap_or("");
        // CX5-C4: the root Cargo.toml too (its `[profile.release]`, the workspace members).
        let fresh_paths: &[&str] = match tool {
            "fuzzilli" => &[
                "crates",
                "Cargo.toml",
                "Cargo.lock",
                "rust-toolchain.toml",
                REGRESS_PATHS[0],
                REGRESS_PATHS[1],
            ],
            "cargo-fuzz" => &CARGO_FUZZ_FRESH_PATHS,
            _ => {
                status = Status::Fail;
                why.push(format!("{}: unknown tool {tool:?}", p.display()));
                continue;
            }
        };
        let counted = j.get("counted").and_then(Json::bool).unwrap_or(false);
        let campaign = j
            .get("campaign")
            .and_then(Json::str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        // A second record of a counted campaign adds no hours, but its open crashes still count
        // (a crash found late in a campaign is still open). F4 (in:190 item 5): the id is
        // inserted below, only once this record counts, so a record of the id that is not
        // counted (dirty, stale, no `dirty` field) does not block a later one that counts.
        let key = campaign.map(|id| format!("{tool}/{id}"));
        let dup = counted && key.as_ref().is_some_and(|k| campaigns.contains(k));
        if dup {
            why.push(format!(
                "{}: campaign {tool} {:?} again, its hours counted once",
                p.display(),
                campaign.unwrap_or("")
            ));
        }
        for cr in j.get("crashes").and_then(Json::arr).unwrap_or(&[]) {
            let state = cr.get("state").and_then(Json::str).unwrap_or("open");
            if !CLOSED.contains(&state) {
                let crash = format!(
                    "{} ({tool}, {state})",
                    cr.get("id").and_then(Json::str).unwrap_or("?")
                );
                if !open.contains(&crash) {
                    open.push(crash);
                }
            }
        }
        let h = j.get("cpu_hours").and_then(Json::num).unwrap_or(0.0);
        let commit = j.get("commit").and_then(Json::str).unwrap_or("unknown");
        let counted = counted
            && !dup
            && match campaign {
                Some(_) => true,
                None => {
                    status = status.max(Status::Open);
                    why.push(format!(
                        "{}: not counted: a counted record names no campaign id",
                        p.display()
                    ));
                    false
                }
            };
        // CX6-1 (replaces the CX5-C5 NOTE rule): only `dirty: false` is counted; `true`, no
        // field or a non-bool gives OPEN (the ep9-fuzz/v1 producers add the field in month 5).
        let counted = counted
            && match j.get("dirty").and_then(Json::bool) {
                Some(false) => true,
                Some(true) => {
                    status = status.max(Status::Open);
                    why.push(format!(
                        "{}: not counted: made with uncommitted changes (dirty)",
                        p.display()
                    ));
                    false
                }
                None => {
                    status = status.max(Status::Open);
                    why.push(format!(
                        "{}: not counted: it does not say whether its tree was dirty (no `dirty` bool)",
                        p.display()
                    ));
                    false
                }
            };
        if counted {
            match fresh_named(&c.o.root, commit, fresh_paths) {
                Ok(_) => {
                    // F4: the campaign counts from here on (`counted` needs a campaign id).
                    campaigns.extend(key);
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
            "{} {tool}{} {:.1} CPU-h at `{}`{}",
            p.display(),
            campaign.map_or(String::new(), |id| format!(" campaign {id:?}")),
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
            let src = match Source::of_log(log) {
                Ok(s) => source_check(c, &s, &T262_FRESH_PATHS),
                Err(e) => (vec![format!("provenance: {e}")], "not read".into()),
            };
            with_source(
                cell(status, format!("{}: {}", p.display(), parts.join(", "))),
                src,
            )
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
            "stub markers (plan v9 and v10 rule 4)",
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
                "- {CI_TRIPLE} column: CI run {} on commit `{}`{} ({} jobs); {}.",
                ci.id.as_deref().unwrap_or("(no id)"),
                ci.head.as_deref().unwrap_or("unknown"),
                ci.url
                    .as_deref()
                    .map(|u| format!(", {u}"))
                    .unwrap_or_default(),
                ci.jobs.len(),
                match &c.ci_fresh {
                    Ok(t) => format!("fresh: {t}"),
                    Err(why) => format!("stale (every cell at least OPEN): {why}"),
                }
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

    /// A fake repo (a git work tree) with the fixture ADR README and spike review, the five fresh
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
        write(&root, "Cargo.toml", "[workspace]\n");
        write(&root, "Cargo.lock", "# lock\n");
        write(&root, "rust-toolchain.toml", "[toolchain]\n");
        write(&root, "docs/decisions-log.md", "# log\n");
        write(&root, "third_party/test262.commit", &format!("{PIN}\n"));
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

    /// The stub-marker row reads the m2, m3, m4 and m5 markers, in the rule-4 dirs.
    #[test]
    fn the_stub_row_reads_four_markers() {
        let root = fake_repo("stubs");
        let r = run(&opts(&root)).unwrap();
        assert_eq!(
            r.extra[1].1.evidence,
            format!(
                "no `{}`, `{}`, `{}` or `{}` line",
                STUB_MARKERS[0], STUB_MARKERS[1], STUB_MARKERS[2], STUB_MARKERS[3]
            )
        );
        for (i, f) in ["crates/a.rs", "spikes/b.rs", "tools/c.rs", "crates/e.rs"]
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
        assert!(c.evidence.starts_with("4 stub line(s)"), "{}", c.evidence);
        for f in [
            "crates/a.rs:1",
            "spikes/b.rs:1",
            "tools/c.rs:1",
            "crates/e.rs:1",
        ] {
            assert!(c.evidence.contains(f), "{f}: {}", c.evidence);
        }
        for f in ["crates/a.rs", "spikes/b.rs", "tools/c.rs", "crates/e.rs"] {
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

    /// The source fields of a runner JSON at `rev` of `root` (clean tree, a host named), with
    /// the trailing comma.
    /// The test262 pin of [`fake_repo`].
    const PIN: &str = "a111111111111111111111111111111111111111";

    /// The source fields of an ep9-test262 JSON made at `rev`, by a runner built there, with the
    /// pinned test262 checkout.
    /// The build fields of a gate-build ep9-test262 JSON (X3).
    const GATE_BUILD: &str =
        r#""mode":"jit","preset":"jit","features":["jit","become"],"feedback":"full","#;
    /// The `--version` build line of a gate-build ep9 (X3).
    const GATE_LINE: &str = "features jit,become; preset jit";

    fn src_at(root: &Path, rev: &str) -> String {
        let commit = git_in(root, &["rev-parse", rev]);
        format!(
            r#""commit":"{commit}","dirty":false,"commit_source":"git","made_by":"host:test","runner_commit":"{commit}","runner_dirty":false,"test262_head":"{PIN}","test262_head_end":"{PIN}","test262_dirty":false,{GATE_BUILD}"#,
        )
    }

    fn exec_json(lp: u64, lt: u64, total: u64, gc: f64, fails: &[&str], src: &str) -> String {
        format!(
            r#"{{"command":"exec","target":"{HOST}",{src}"gc_stress":{gc},"files":{{"passed":{},"total":{total}}},"by_dir":{{"language":{{"files":{{"passed":{lp},"total":{lt}}}}}}},"results":[{}]}}"#,
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
            o.exec = Some(input(
                &root,
                "exec.json",
                &exec_json(lp, lt, 24, gc, &[], &src_at(&root, "HEAD")),
            ));
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
        // The fixture has no source line: the tests pass, but the input is stale (P3-ORCH-14).
        o.nesting = Some(fixture("nesting.log"));
        let c = &run(&o).unwrap().cells[7].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence.contains("the 9 tests ok, 2 cargo runs"),
            "{}",
            c.evidence
        );
        assert!(
            c.evidence.contains("stale input: it names no commit"),
            "{}",
            c.evidence
        );
        let log = with_source_line(
            &read(&fixture("nesting.log")).unwrap(),
            &git_in(&root, &["rev-parse", "HEAD"]),
            0,
        );
        o.nesting = Some(input(&root, "nesting.log", &log));
        let c = &run(&o).unwrap().cells[7].0;
        assert_eq!(c.status, Status::Pass, "{c:?}");
        assert!(c.evidence.contains("source: commit `"), "{}", c.evidence);
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

    /// The nesting log with the NESTING_CMD source line after its first start line.
    fn with_source_line(log: &str, commit: &str, dirty: u32) -> String {
        log.replacen(
            "m0-exit start: depth_tests\n",
            &format!(
                "m0-exit start: depth_tests\n{SOURCE_LINE}commit {commit} dirty {dirty} made-by host:test\n"
            ),
            1,
        )
    }

    /// P3-ORCH-14 (finding 6): item 8 needs a fresh, clean, named nesting log.
    #[test]
    fn item8_needs_a_fresh_named_log() {
        let root = fake_repo("item8src");
        let first = git_in(&root, &["rev-parse", "HEAD"]);
        let mut o = opts(&root);
        let log = read(&fixture("nesting.log")).unwrap();
        // A later commit outside the fresh paths keeps it fresh.
        write(&root, "docs/x.md", "doc\n");
        commit_all(&root, "docs");
        let head = git_in(&root, &["rev-parse", "HEAD"]);
        let two = with_source_line(&log, &first, 0).replace(
            "m0-exit start: nesting_caps cli\n",
            &format!("m0-exit start: nesting_caps cli\n{SOURCE_LINE}commit {head} dirty 0 made-by host:test\n"),
        );
        for (text, want, why) in [
            (
                with_source_line(&log, &first, 0),
                Status::Pass,
                "an ancestor of HEAD",
            ),
            (
                with_source_line(&log, &head, 0),
                Status::Pass,
                "this tree (HEAD)",
            ),
            (
                with_source_line(&log, &head, 3),
                Status::Open,
                "stale input: measured with uncommitted changes",
            ),
            (
                with_source_line(&log, &format!("0123abc{}", "0".repeat(33)), 0),
                Status::Open,
                "stale input: its commit `0123abc000000000000000000000000000000000` is not in this repository",
            ),
            // A ref name is not a commit id, even when it resolves (verifier r1 F3).
            (
                with_source_line(&log, "HEAD", 0),
                Status::Open,
                "stale input: its commit \"HEAD\" is not a commit id (40 hex digits)",
            ),
            (
                with_source_line(&log, &head[..12], 0),
                Status::Open,
                "is not a commit id (40 hex digits)",
            ),
            (
                two,
                Status::Open,
                "provenance: two source lines that differ",
            ),
            // Two trees lines that differ (verifier r1 F5).
            (
                with_source_line(&log, &head, 0)
                    .replacen(
                        "m0-exit start: depth_tests\n",
                        &format!("m0-exit start: depth_tests\n{TREES_LINE}crates=aaa\n"),
                        1,
                    )
                    .replacen(
                        "m0-exit start: nesting_caps cli\n",
                        &format!("m0-exit start: nesting_caps cli\n{TREES_LINE}crates=bbb\n"),
                        1,
                    ),
                Status::Open,
                "provenance: two trees lines that differ",
            ),
            (
                log.replacen(
                    "m0-exit start: depth_tests\n",
                    &format!("m0-exit start: depth_tests\n{SOURCE_LINE}{head}\n"),
                    1,
                ),
                Status::Open,
                "a source line in another form",
            ),
            (
                with_source_line(&log, &head, 0).replace(" made-by host:test", " made-by "),
                Status::Open,
                "a source line in another form",
            ),
        ] {
            o.nesting = Some(input(&root, "nesting.log", &text));
            let c = &run(&o).unwrap().cells[7].0;
            assert_eq!(c.status, want, "{why}: {c:?}");
            assert!(c.evidence.contains(why), "{why}: {}", c.evidence);
        }
        // A change in tools/test262 (or crates, ...) after the log's commit: stale.
        write(&root, "tools/test262/src/main.rs", "// runner\n");
        commit_all(&root, "runner");
        o.nesting = Some(input(
            &root,
            "nesting.log",
            &with_source_line(&log, &head, 0),
        ));
        let c = &run(&o).unwrap().cells[7].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence.contains("stale input: not this tree: crates, tools/bench, Cargo.toml, Cargo.lock, rust-toolchain.toml, tools/test262, third_party/test262.commit, third_party/regress.commit, third_party/patches/regress changed"),
            "{}",
            c.evidence
        );
        // So does a new test262 pin.
        let head = git_in(&root, &["rev-parse", "HEAD"]);
        write(&root, "third_party/test262.commit", "abc\n");
        commit_all(&root, "pin");
        o.nesting = Some(input(
            &root,
            "nesting.log",
            &with_source_line(&log, &head, 0),
        ));
        assert_eq!(run(&o).unwrap().cells[7].0.status, Status::Open);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// M4-P5 C1: the root `Cargo.toml` (its `[profile.release]` sets `lto` and `panic`) is a fresh
    /// path of every list: a change to it after the input's commit makes items 1, 2 and 8 OPEN,
    /// and (CX5-C4) item 5: neither fuzz record counts.
    #[test]
    fn a_root_cargo_toml_change_makes_inputs_stale() {
        for list in [&FRESH_PATHS[..], &T262_FRESH_PATHS[..], &CI_FRESH_PATHS[..]] {
            assert!(list.contains(&"Cargo.toml"), "{list:?}");
        }
        assert!(
            NESTING_CMD.contains(" Cargo.toml Cargo.lock "),
            "{NESTING_CMD}"
        );
        let root = fake_repo("cargotoml");
        let head = git_in(&root, &["rev-parse", "HEAD"]);
        let mut o = opts(&root);
        let log = with_source_line(&read(&fixture("nesting.log")).unwrap(), &head, 0);
        o.kernels = Some(input(
            &root,
            "kernels.json",
            &kernels_json(&root, HOST, 15, 15, None),
        ));
        o.cross = Some(input(
            &root,
            "exit2.json",
            &exit2_json(&head, ["pass", "pass"], true),
        ));
        o.nesting = Some(input(&root, "nesting.log", &log));
        // CX5-C4: the fuzz records of item 5 too (both tools).
        o.fuzz = vec![
            input(
                &root,
                "f1.json",
                &fuzz_json("fuzzilli", &head, 100.0, true, ""),
            ),
            input(
                &root,
                "f2.json",
                &fuzz_json("cargo-fuzz", &head, 24.0, true, ""),
            ),
        ];
        let r = run(&o).unwrap();
        for i in [0, 1, 4, 7] {
            assert_eq!(
                r.cells[i].0.status,
                Status::Pass,
                "item {}: {:?}",
                i + 1,
                r.cells[i].0
            );
        }
        write(
            &root,
            "Cargo.toml",
            "[workspace]\n[profile.release]\nlto = \"fat\"\n",
        );
        commit_all(&root, "profile");
        let r = run(&o).unwrap();
        for (i, why) in [
            (0, "Cargo.toml differs from HEAD"),
            (1, "Cargo.toml"),
            (
                4,
                "f1.json: not counted: not this tree: crates, Cargo.toml, Cargo.lock, rust-toolchain.toml, third_party/regress.commit, third_party/patches/regress changed",
            ),
            (
                4,
                "f2.json: not counted: not this tree: crates/ep9_frontend, crates/ep9_bytecode, fuzz, Cargo.toml, Cargo.lock changed",
            ),
            (7, "Cargo.toml"),
        ] {
            let c = &r.cells[i].0;
            assert_eq!(c.status, Status::Open, "item {}: {c:?}", i + 1);
            assert!(c.evidence.contains(why), "item {}: {}", i + 1, c.evidence);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// pre medium 1 (D41, R41): the regress fork is engine code (ep9_vm builds it through the
    /// `[patch.crates-io]` path), so a change of its pin `third_party/regress.commit` or of a
    /// patch under `third_party/patches/regress` after the input's commit makes items 1, 2, 3
    /// and 8 OPEN and a counted Fuzzilli record not counted. A counted cargo-fuzz record stays
    /// counted: its targets do not build regress.
    #[test]
    fn a_regress_pin_or_patch_change_makes_inputs_stale() {
        for (k, path) in REGRESS_PATHS.iter().enumerate() {
            let root = fake_repo(&format!("regress{k}"));
            write(
                &root,
                "third_party/regress.commit",
                &format!("{}\n", "b".repeat(40)),
            );
            write(
                &root,
                "third_party/patches/regress/README.md",
                "# patches\n",
            );
            let head = commit_all(&root, "regress");
            let mut o = opts(&root);
            o.kernels = Some(input(
                &root,
                "kernels.json",
                &kernels_json(&root, HOST, 15, 15, None),
            ));
            o.cross = Some(input(
                &root,
                "exit2.json",
                &exit2_json(&head, ["pass", "pass"], true),
            ));
            o.exec = Some(input(
                &root,
                "exec.json",
                &exec_json(17, 20, 24, 0.0, &[], &src_at(&root, "HEAD")),
            ));
            let log = with_source_line(&read(&fixture("nesting.log")).unwrap(), &head, 0);
            o.nesting = Some(input(&root, "nesting.log", &log));
            o.fuzz = vec![
                input(
                    &root,
                    "f1.json",
                    &fuzz_json("fuzzilli", &head, 100.0, true, ""),
                ),
                input(
                    &root,
                    "f2.json",
                    &fuzz_json("cargo-fuzz", &head, 24.0, true, ""),
                ),
            ];
            let r = run(&o).unwrap();
            for i in [0, 1, 2, 4, 7] {
                let c = &r.cells[i].0;
                assert_eq!(c.status, Status::Pass, "{path}: item {}: {c:?}", i + 1);
            }
            if k == 0 {
                write(&root, path, &format!("{}\n", "c".repeat(40)));
            } else {
                write(&root, &format!("{path}/0001-fix.patch"), "--- a\n+++ b\n");
            }
            commit_all(&root, path);
            let r = run(&o).unwrap();
            for i in [0, 1, 2, 4, 7] {
                let c = &r.cells[i].0;
                assert_eq!(c.status, Status::Open, "{path}: item {}: {c:?}", i + 1);
                assert!(
                    c.evidence.contains(path),
                    "{path}: item {}: {}",
                    i + 1,
                    c.evidence
                );
            }
            let fuzz = &r.cells[4].0.evidence;
            assert!(
                fuzz.contains("f1.json: not counted: not this tree"),
                "{fuzz}"
            );
            assert!(!fuzz.contains("f2.json: not counted"), "{fuzz}");
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    /// pre medium 1: the runner's JSON names the ids of the paths of [`T262_FRESH_PATHS`]
    /// (`TREE_PATHS` of tools/test262/src/provenance.rs), so a CI merge-commit run is checked on
    /// all of them, the regress fork too. Reads the live source (the two lists must be the same).
    #[test]
    fn the_runner_tree_paths_are_the_t262_fresh_paths() {
        let src = read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../test262/src/provenance.rs"))
            .unwrap();
        let start = src.find("pub const TREE_PATHS").expect("TREE_PATHS");
        let body = &src[start..start + src[start..].find("];").unwrap()];
        let listed: Vec<&str> = body
            .lines()
            .skip(1)
            .filter_map(|l| l.trim().strip_prefix('"')?.strip_suffix("\","))
            .collect();
        assert_eq!(listed, T262_FRESH_PATHS);
    }

    /// The `path = "..."` dependencies of a Cargo.toml text, in its build tables (`[dependencies]`,
    /// `[build-dependencies]` and their `[target.*]` forms; dev-dependencies are not built into a
    /// fuzz target).
    fn path_deps(manifest: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut build_table = false;
        for l in manifest.lines().map(str::trim) {
            if l.starts_with('[') {
                let t = l.trim_matches(['[', ']']);
                build_table = (t.ends_with("dependencies") && !t.ends_with("dev-dependencies"))
                    || (t.contains("dependencies.") && !t.contains("dev-dependencies."));
                continue;
            }
            if !build_table || l.starts_with('#') {
                continue;
            }
            // `name = { path = "<p>", ... }` or, in a `[dependencies.<name>]` table, `path = "<p>"`:
            // a `path` key starts the line or follows `{`, `,` or a space.
            for (k, _) in l.match_indices("path") {
                if !(k == 0 || l[..k].ends_with(['{', ',', ' '])) {
                    continue;
                }
                let value = l[k + "path".len()..]
                    .trim_start()
                    .strip_prefix('=')
                    .map(str::trim_start)
                    .and_then(|r| r.strip_prefix('"'))
                    .and_then(|r| r.split('"').next());
                if let Some(v) = value {
                    out.push(v.to_string());
                    break;
                }
            }
        }
        out
    }

    /// F2 (plan v10 section 2.6, X1): every engine crate that `fuzz/Cargo.toml` builds, through
    /// its path dependencies and theirs, is in [`CARGO_FUZZ_FRESH_PATHS`]; so a change in any of
    /// them makes a counted cargo-fuzz record stale. Reads the live manifests (the list must
    /// follow them).
    #[test]
    fn the_cargo_fuzz_fresh_list_names_every_path_dependency() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let repo = std::fs::canonicalize(&repo).unwrap();
        let mut todo = vec![repo.join("fuzz")];
        let mut seen: BTreeSet<String> = BTreeSet::new();
        while let Some(dir) = todo.pop() {
            let text = read(&dir.join("Cargo.toml")).unwrap();
            for dep in path_deps(&text) {
                let d = std::fs::canonicalize(dir.join(&dep))
                    .unwrap_or_else(|e| panic!("{}: {dep}: {e}", dir.display()));
                let rel = d
                    .strip_prefix(&repo)
                    .unwrap_or_else(|_| panic!("{dep} is outside the repo"))
                    .to_string_lossy()
                    .replace('\\', "/");
                if seen.insert(rel) {
                    todo.push(d);
                }
            }
        }
        assert!(seen.contains("crates/ep9_frontend"), "{seen:?}");
        let missing: Vec<&String> = seen
            .iter()
            .filter(|p| !CARGO_FUZZ_FRESH_PATHS.contains(&p.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "fuzz/Cargo.toml builds {missing:?}, which CARGO_FUZZ_FRESH_PATHS lacks"
        );
        // The parser reads both forms and skips dev-dependencies.
        assert_eq!(
            path_deps(
                "[dependencies]\na = { path = \"../a\" }\nb = \"1\"\n[dev-dependencies]\n\
                 c = { path = \"../c\" }\n[dependencies.d]\npath = \"../d\"\n\
                 [target.'cfg(unix)'.dependencies]\ne = { version = \"1\", path = \"../e\" }\n\
                 [dependencies.pathx]\npath = \"../pathx\"\n"
            ),
            ["../a", "../d", "../e", "../pathx"]
        );
    }

    /// The CI nesting step (.github/workflows/ci.yml) writes the two NESTING_CMD source lines
    /// right after its first start line, with the same text, so the linux-a64 CI log of item 8
    /// names its source (gates need 1).
    #[test]
    fn ci_yml_writes_the_nesting_cmd_source_lines() {
        let ci =
            read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows/ci.yml"))
                .unwrap();
        let lines: Vec<&str> = ci.lines().map(str::trim).collect();
        let start = lines
            .iter()
            .position(|l| *l == r#"echo "m0-exit start: depth_tests" | tee nesting.log"#)
            .expect("the nesting step's first start line");
        // Each `echo "m1-exit ...` command of NESTING_CMD, up to its ` >> nesting.log`.
        let want: Vec<String> = NESTING_CMD
            .match_indices("echo \"m1-exit ")
            .map(|(k, _)| {
                let c = &NESTING_CMD[k..];
                let end = c.find(" >> nesting.log").unwrap_or_else(|| panic!("{c}"));
                format!("{} | tee -a nesting.log", &c[..end])
            })
            .collect();
        assert_eq!(want.len(), 2, "{NESTING_CMD}");
        assert_eq!(
            lines[start + 1..start + 3],
            want[..],
            "ci.yml after line {}",
            start + 1
        );
    }

    /// pre medium 1: the trees line of NESTING_CMD names exactly [`T262_FRESH_PATHS`], so a CI
    /// merge-commit log passes on the ids of every path item 8 checks (the regress fork too).
    #[test]
    fn the_nesting_trees_line_names_the_t262_fresh_paths() {
        let k = NESTING_CMD.find("for p in ").expect("the trees loop") + "for p in ".len();
        let list = &NESTING_CMD[k..k + NESTING_CMD[k..].find(';').unwrap()];
        assert_eq!(list.split(' ').collect::<Vec<_>>(), T262_FRESH_PATHS);
    }

    /// The two NESTING_CMD lines that name the source, run by `sh` in a fake repo: item 8 reads
    /// them as this tree, clean, with its path ids; a CI merge commit that the repo does not have
    /// passes on those ids, and fails when one id differs.
    #[cfg(unix)]
    #[test]
    fn the_nesting_cmd_source_lines_work() {
        let root = fake_repo("nestcmd");
        // NESTING_CMD up to its first cargo run: the start line and the two source lines.
        let prefix = &NESTING_CMD[..NESTING_CMD.find("cargo test").unwrap()];
        let out = Command::new("sh")
            .arg("-c")
            .arg(prefix.trim().trim_end_matches(';'))
            .current_dir(&root)
            .env_remove("GITHUB_RUN_ID")
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let made = read(&root.join("nesting.log")).unwrap();
        std::fs::remove_file(root.join("nesting.log")).unwrap();
        let text = made
            .strip_prefix("m0-exit start: depth_tests\n")
            .unwrap_or_else(|| panic!("{made}"))
            .to_string();
        assert_eq!(text.lines().count(), 2, "{text}");
        let head = git_in(&root, &["rev-parse", "HEAD"]);
        assert!(
            text.starts_with(&format!("{SOURCE_LINE}commit {head} dirty 0 made-by host:")),
            "{text}"
        );
        let src = Source::of_log(&text).unwrap();
        // The fake repo has 6 of the 7 paths, the root Cargo.toml among them (C1).
        assert_eq!(src.trees.len(), 6, "{src:?}");
        assert_eq!(
            src.trees.get("Cargo.toml"),
            Some(&git_in(&root, &["rev-parse", "HEAD:Cargo.toml"]))
        );
        assert_eq!(
            src.trees.get("Cargo.lock"),
            Some(&git_in(&root, &["rev-parse", "HEAD:Cargo.lock"]))
        );
        let log = read(&fixture("nesting.log")).unwrap().replacen(
            "m0-exit start: depth_tests\n",
            &format!("m0-exit start: depth_tests\n{text}"),
            1,
        );
        let mut o = opts(&root);
        o.nesting = Some(input(&root, "nesting.log", &log));
        let c = &run(&o).unwrap().cells[7].0;
        assert_eq!(c.status, Status::Pass, "{c:?}");
        // A commit the repo does not have (GitHub's pull_request merge commit), same ids: fresh.
        let merge = "e".repeat(40);
        let foreign = log.replace(&format!("commit {head} "), &format!("commit {merge} "));
        o.nesting = Some(input(&root, "nesting.log", &foreign));
        let c = &run(&o).unwrap().cells[7].0;
        assert_eq!(c.status, Status::Pass, "{c:?}");
        assert!(
            c.evidence.contains("but its ids of crates, tools/bench"),
            "{}",
            c.evidence
        );
        // ... but not when one id differs.
        let lock = git_in(&root, &["rev-parse", "HEAD:Cargo.lock"]);
        o.nesting = Some(input(
            &root,
            "nesting.log",
            &foreign.replace(&lock, &"0".repeat(40)),
        ));
        let c = &run(&o).unwrap().cells[7].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence.contains("is not in this repository"),
            "{}",
            c.evidence
        );
        // Nor with no trees line.
        let bare: String = foreign
            .lines()
            .filter(|l| !l.starts_with(TREES_LINE))
            .map(|l| format!("{l}\n"))
            .collect();
        o.nesting = Some(input(&root, "nesting.log", &bare));
        assert_eq!(run(&o).unwrap().cells[7].0.status, Status::Open);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The exec JSON of a CI merge commit: fresh when its `tree` ids equal HEAD's.
    #[test]
    fn item3_reads_tree_ids_of_a_foreign_commit() {
        let root = fake_repo("item3tree");
        let mut o = opts(&root);
        let ids: Vec<String> = [
            "crates",
            "tools/bench",
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "third_party/test262.commit",
        ]
        .iter()
        .map(|p| {
            format!(
                r#""{p}":"{}""#,
                git_in(&root, &["rev-parse", &format!("HEAD:{p}")])
            )
        })
        .collect();
        let src = |ids: &str| {
            format!(
                r#""commit":"{e}","dirty":false,"made_by":"host:ci gh-run:1/build","runner_commit":"{e}","runner_dirty":false,"test262_head":"{PIN}","test262_head_end":"{PIN}","test262_dirty":false,{GATE_BUILD}"tree":{{{ids}}},"#,
                e = "e".repeat(40)
            )
        };
        o.exec = Some(input(
            &root,
            "exec.json",
            &exec_json(17, 20, 24, 0.0, &[], &src(&ids.join(","))),
        ));
        let c = &run(&o).unwrap().cells[2].0;
        assert_eq!(c.status, Status::Pass, "{c:?}");
        // One path missing from the ids, or one more (a path HEAD does not have): stale.
        for bad in [
            ids[1..].join(","),
            format!(r#"{},"tools/test262":"abc""#, ids.join(",")),
        ] {
            o.exec = Some(input(
                &root,
                "exec.json",
                &exec_json(17, 20, 24, 0.0, &[], &src(&bad)),
            ));
            let c = &run(&o).unwrap().cells[2].0;
            assert_eq!(c.status, Status::Open, "{bad}: {c:?}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// P3-ORCH-14 and P3-ORCH-21: the exec JSON of item 3 names a fresh commit, a clean tree and
    /// its host; a job of another CI run than the one given is a mismatch.
    #[test]
    fn item3_needs_a_fresh_named_exec_json() {
        let root = fake_repo("item3src");
        let first = git_in(&root, &["rev-parse", "HEAD"]);
        write(&root, "docs/x.md", "doc\n");
        commit_all(&root, "docs");
        let mut o = opts(&root);
        let ok = src_at(&root, "HEAD");
        let head = git_in(&root, &["rev-parse", "HEAD"]);
        let cases = [
            (ok.clone(), Status::Pass, "source: commit `"),
            (src_at(&root, &first), Status::Pass, "an ancestor of HEAD"),
            (
                String::new(),
                Status::Open,
                "stale input: it names no commit",
            ),
            (
                ok.replace(r#""dirty":false"#, r#""dirty":true"#),
                Status::Open,
                "stale input: measured with uncommitted changes (dirty)",
            ),
            (
                ok.replace(r#""dirty":false,"#, ""),
                Status::Open,
                "stale input: it does not say whether the tree was dirty",
            ),
            // CX5-C1: the runner read another source state at the end of the run than at its
            // start (it then writes dirty true too).
            (
                ok.replace(
                    r#""dirty":false,"#,
                    r#""dirty":true,"source_changed_during_run":true,"#,
                ),
                Status::Open,
                "stale input: the source changed during the run (source_changed_during_run)",
            ),
            // F3: the runner's test seam (tools/test262/tests/exec.rs) is no measurement.
            (
                ok.replace(r#""dirty":false,"#, r#""dirty":false,"test_seam":true,"#),
                Status::Open,
                "provenance: the runner read its source through its test seam (test_seam)",
            ),
            (
                format!(
                    r#""commit":"{}","dirty":false,"made_by":"host:t","#,
                    "f".repeat(40)
                ),
                Status::Open,
                "is not in this repository",
            ),
            (
                ok.replace(r#""made_by":"host:test","#, ""),
                Status::Open,
                "provenance: it names no host or job (made_by)",
            ),
            // A ref name is not a commit id (verifier r1 F3).
            (
                r#""commit":"HEAD","dirty":false,"made_by":"host:t","#.to_string(),
                Status::Open,
                "stale input: its commit \"HEAD\" is not a commit id",
            ),
            // X4 (plan v10 section 2.6): a runner built at the input's commit, a test262
            // checkout at the pin; a missing fact is OPEN too.
            (
                ok.replace(&format!(r#""runner_commit":"{head}","#), ""),
                Status::Open,
                "provenance: it names no runner build commit (runner_commit;",
            ),
            (
                ok.replace(
                    &format!(r#""runner_commit":"{head}""#),
                    &format!(r#""runner_commit":"{first}""#),
                ),
                Status::Open,
                "provenance: the runner was built at `",
            ),
            (
                ok.replace(&format!(r#""test262_head":"{PIN}","#), ""),
                Status::Open,
                "provenance: it names no test262 checkout HEAD (test262_head;",
            ),
            (
                ok.replace(PIN, &"2".repeat(40)),
                Status::Open,
                "provenance: the test262 checkout was at `22222222…22222`, the pin (third_party/test262.commit) is `a1111111…11111`",
            ),
            // Upper-case hex is the same id.
            (
                ok.replace(PIN, &PIN.to_ascii_uppercase()),
                Status::Pass,
                "source: commit `",
            ),
            // gates5 medium 5: a build fact that is not a full commit id is OPEN, also when it
            // holds non-ASCII text (the shortening of the evidence text must not panic).
            (
                ok.replace(
                    &format!(r#""runner_commit":"{head}""#),
                    &format!(r#""runner_commit":"{}""#, "\u{e9}".repeat(10)),
                ),
                Status::Open,
                "provenance: its runner_commit \"\u{e9}\u{e9}\u{e9}",
            ),
            (
                ok.replace(
                    &format!(r#""runner_commit":"{head}""#),
                    r#""runner_commit":"abc123""#,
                ),
                Status::Open,
                "provenance: its runner_commit \"abc123\" is not a commit id (40 hex digits)",
            ),
            (
                ok.replace(PIN, &"\u{e9}".repeat(10)),
                Status::Open,
                "provenance: its test262_head \"\u{e9}\u{e9}\u{e9}",
            ),
            (
                ok.replace(PIN, &PIN[..12]),
                Status::Open,
                "provenance: its test262_head \"a11111111111\" is not a commit id (40 hex digits)",
            ),
            // gates5 high 1: a runner built from uncommitted changes of its compiled sources,
            // or one that does not say, is OPEN.
            (
                ok.replace(r#""runner_dirty":false"#, r#""runner_dirty":true"#),
                Status::Open,
                "provenance: the runner was built from uncommitted changes (runner_dirty)",
            ),
            (
                ok.replace(r#""runner_dirty":false,"#, ""),
                Status::Open,
                "provenance: it names no runner dirty flag (runner_dirty;",
            ),
            // gates5 high 2: a test262 checkout with changes (before or after the run), one
            // whose HEAD moved during the run, or a JSON that does not say, is OPEN.
            (
                ok.replace(r#""test262_dirty":false"#, r#""test262_dirty":true"#),
                Status::Open,
                "provenance: the test262 checkout had changes (test262_dirty)",
            ),
            (
                ok.replace(r#""test262_dirty":false,"#, ""),
                Status::Open,
                "provenance: it names no test262 dirty flag (test262_dirty;",
            ),
            (
                ok.replace(
                    &format!(r#""test262_head_end":"{PIN}""#),
                    &format!(r#""test262_head_end":"{}""#, "3".repeat(40)),
                ),
                Status::Open,
                "provenance: the test262 checkout moved during the run, from `a1111111\u{2026}11111` to `33333333\u{2026}33333`",
            ),
            (
                ok.replace(&format!(r#""test262_head_end":"{PIN}","#), ""),
                Status::Open,
                "provenance: it names no test262 checkout HEAD at the end of the run (test262_head_end;",
            ),
        ];
        for (src, want, text) in cases {
            o.exec = Some(input(
                &root,
                "exec.json",
                &exec_json(17, 20, 24, 0.0, &[], &src),
            ));
            let c = &run(&o).unwrap().cells[2].0;
            assert_eq!(c.status, want, "{text}: {c:?}");
            assert!(c.evidence.contains(text), "{text}: {}", c.evidence);
        }
        // A runner change after the run: stale.
        write(&root, "tools/test262/src/exec.rs", "// runner\n");
        commit_all(&root, "runner");
        o.exec = Some(input(
            &root,
            "exec.json",
            &exec_json(17, 20, 24, 0.0, &[], &ok),
        ));
        let c = &run(&o).unwrap().cells[2].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence.contains("stale input: not this tree"),
            "{}",
            c.evidence
        );
        // A JSON made by CI run 999, read next to CI run 123: a mismatch.
        let head = src_at(&root, "HEAD");
        let by_ci = head.replace("host:test", "host:runner gh-run:999/build");
        o.exec = Some(input(
            &root,
            "exec.json",
            &exec_json(17, 20, 24, 0.0, &[], &by_ci),
        ));
        o.ci_run_json = Some(fixture("ci-run.json"));
        let c = &run(&o).unwrap().cells[2].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence
                .contains("provenance: made by CI run 999, but the CI run given is 123"),
            "{}",
            c.evidence
        );
        let by_ci = head.replace("host:test", "host:runner gh-run:123/build");
        o.exec = Some(input(
            &root,
            "exec.json",
            &exec_json(17, 20, 24, 0.0, &[], &by_ci),
        ));
        assert_eq!(run(&o).unwrap().cells[2].0.status, Status::Pass);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// P3-ORCH-21: the GC-stress inputs name one commit, the --exec input the same one.
    #[test]
    fn item4_inputs_name_one_tree() {
        let root = fake_repo("item4src");
        let first = git_in(&root, &["rev-parse", "HEAD"]);
        write(&root, "docs/x.md", "doc\n");
        commit_all(&root, "docs");
        let head = src_at(&root, "HEAD");
        let old = src_at(&root, &first);
        let mut o = opts(&root);
        let gc = |src: &str| gc_json(&["test/language/l00.js"], S1, src);
        let exec = |src: &str| {
            exec_json(
                18,
                20,
                24,
                0.0,
                &["test/language/l00.js", "test/language/l01.js"],
                src,
            )
        };
        o.exec = Some(input(&root, "exec.json", &exec(&head)));
        o.exec_gc = vec![input(&root, "gc.json", &gc(&head))];
        let c = &run(&o).unwrap().cells[3].0;
        assert_eq!(c.status, Status::Pass, "{c:?}");
        // --exec of an older (still fresh) commit than the GC run: not one tree.
        o.exec = Some(input(&root, "exec.json", &exec(&old)));
        let c = &run(&o).unwrap().cells[3].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence.contains("the same pass set needs one tree"),
            "{}",
            c.evidence
        );
        // Two GC inputs of two commits.
        o.exec = None;
        o.exec_gc = vec![
            input(&root, "gc1.json", &gc(&head)),
            input(&root, "gc2.json", &gc(&old)),
        ];
        let c = &run(&o).unwrap().cells[3].0;
        assert!(c.status >= Status::Open, "{c:?}");
        assert!(
            c.evidence
                .contains("provenance: the 2 GC-stress inputs name 2 commits"),
            "{}",
            c.evidence
        );
        // A dirty GC input.
        o.exec_gc = vec![input(
            &root,
            "gc.json",
            &gc(&head.replace(r#""dirty":false"#, r#""dirty":true"#)),
        )];
        o.exec = Some(input(&root, "exec.json", &exec(&head)));
        let c = &run(&o).unwrap().cells[3].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(c.evidence.contains("(dirty)"), "{}", c.evidence);
        // A fresh GC input next to an --exec input with no commit, a ref name, a dirty tree, or
        // no dirty field: not one known tree (verifier r1 F4).
        o.exec_gc = vec![input(&root, "gc.json", &gc(&head))];
        for (src, text) in [
            (String::new(), "names no commit id"),
            (
                r#""commit":"HEAD","dirty":false,"made_by":"host:t","#.to_string(),
                "names no commit id",
            ),
            (
                head.replace(r#""dirty":false"#, r#""dirty":true"#),
                "was measured with uncommitted changes, or does not say",
            ),
            (
                head.replace(r#""dirty":false,"#, ""),
                "was measured with uncommitted changes, or does not say",
            ),
            // F3: an --exec input that the runner made through its test seam (M5-P1 review).
            (
                head.replace(r#""dirty":false,"#, r#""dirty":false,"test_seam":true,"#),
                "read its source through its test seam (test_seam)",
            ),
        ] {
            o.exec = Some(input(&root, "exec.json", &exec(&src)));
            let c = &run(&o).unwrap().cells[3].0;
            assert_eq!(c.status, Status::Open, "{text}: {c:?}");
            assert!(
                c.evidence.contains("provenance: --exec ") && c.evidence.contains(text),
                "{text}: {}",
                c.evidence
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// P3-ORCH-21: the CI run's head is HEAD (or an ancestor with no change in the CI paths);
    /// else each Windows cell is at least OPEN.
    #[test]
    fn a_stale_ci_run_proves_nothing() {
        let root = fake_repo("cihead");
        let first = git_in(&root, &["rev-parse", "HEAD"]);
        let ci = |head: Option<&str>| {
            let head = head.map_or(String::new(), |h| format!(r#""headSha":"{h}","#));
            format!(
                r#"{{"databaseId":5,{head}"jobs":[{{"name":"build + test (windows-x64)","steps":[{{"name":"Run cargo test --workspace --locked","status":"completed","conclusion":"success"}}]}}]}}"#
            )
        };
        let mut o = opts(&root);
        o.ci_run_json = Some(input(&root, "ci.json", &ci(Some(&first))));
        let r = run(&o).unwrap();
        assert_eq!(r.cells[7].1.status, Status::Pass, "{:?}", r.cells[7].1);
        assert!(
            r.markdown.contains("fresh: this tree (HEAD)"),
            "{}",
            r.markdown
        );
        // A docs commit since: still fresh.
        write(&root, "docs/x.md", "doc\n");
        commit_all(&root, "docs");
        assert_eq!(run(&o).unwrap().cells[7].1.status, Status::Pass);
        // An expectation change since: stale; so is a run with no head.
        write(&root, "expectations/x.txt", "x\n");
        commit_all(&root, "expect");
        let r = run(&o).unwrap();
        let w = &r.cells[7].1;
        assert_eq!(w.status, Status::Open, "{w:?}");
        assert!(
            w.evidence
                .starts_with("stale CI run: CI run 5: not this tree"),
            "{}",
            w.evidence
        );
        assert!(r.markdown.contains("stale (every cell at least OPEN)"));
        o.ci_run_json = Some(input(&root, "ci.json", &ci(None)));
        let w = &run(&o).unwrap().cells[7].1;
        assert_eq!(w.status, Status::Open, "{w:?}");
        assert!(
            w.evidence.contains("names no head commit"),
            "{}",
            w.evidence
        );
        // A ref name as headSha is not a commit id, though it resolves to HEAD (verifier r1 F3).
        o.ci_run_json = Some(input(&root, "ci.json", &ci(Some("HEAD"))));
        let w = &run(&o).unwrap().cells[7].1;
        assert_eq!(w.status, Status::Open, "{w:?}");
        assert!(
            w.evidence
                .contains("its headSha \"HEAD\" is not a commit id (40 hex digits)"),
            "{}",
            w.evidence
        );
        // A step that did not run stays NOT-RUN.
        assert_eq!(run(&o).unwrap().cells[0].1.status, Status::NotRun);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// P3-ORCH-21: each fuzz campaign counts once.
    #[test]
    fn item5_counts_each_campaign_once() {
        let root = fake_repo("item5dup");
        let head = git_in(&root, &["rev-parse", "HEAD"]);
        let mut o = opts(&root);
        let half = fuzz_json(
            "fuzzilli",
            &head,
            50.0,
            true,
            r#"{"id":"c1","state":"open"}"#,
        );
        let cargo = fuzz_json("cargo-fuzz", &head, 24.0, true, "");
        // The same file twice.
        let f = input(&root, "half.json", &half);
        o.fuzz = vec![f.clone(), f, input(&root, "cargo.json", &cargo)];
        let c = &run(&o).unwrap().cells[4].0;
        assert!(
            c.evidence.contains("half.json: given twice, counted once"),
            "{}",
            c.evidence
        );
        assert!(
            c.evidence.contains("Fuzzilli 50.0 / 100 CPU-h"),
            "{}",
            c.evidence
        );
        assert!(
            c.evidence.contains("1 open unique crashes"),
            "{}",
            c.evidence
        );
        // Two files of one campaign id.
        o.fuzz = vec![
            input(&root, "a.json", &half),
            input(&root, "b.json", &half),
            input(&root, "cargo.json", &cargo),
        ];
        let c = &run(&o).unwrap().cells[4].0;
        assert!(
            c.evidence.contains(
                "b.json: campaign fuzzilli \"fuzzilli-50\" again, its hours counted once"
            ),
            "{}",
            c.evidence
        );
        assert!(
            c.evidence.contains("Fuzzilli 50.0 / 100 CPU-h"),
            "{}",
            c.evidence
        );
        // Two campaigns of 50 h each: 100 h, PASS (crash closed).
        let a = fuzz_json("fuzzilli", &head, 50.0, true, "");
        let b = a.replace("fuzzilli-50", "fuzzilli-50-b");
        o.fuzz = vec![
            input(&root, "a.json", &a),
            input(&root, "b.json", &b),
            input(&root, "cargo.json", &cargo),
        ];
        let c = &run(&o).unwrap().cells[4].0;
        assert_eq!(c.status, Status::Pass, "{c:?}");
        // A counted record with no campaign id is not counted.
        o.fuzz = vec![
            input(
                &root,
                "a.json",
                &fuzz_json("fuzzilli", &head, 100.0, true, "")
                    .replace(r#""campaign":"fuzzilli-100","#, ""),
            ),
            input(&root, "cargo.json", &cargo),
        ];
        let c = &run(&o).unwrap().cells[4].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence.contains("a counted record names no campaign id"),
            "{}",
            c.evidence
        );
        assert!(c.evidence.contains("Fuzzilli 0.0 / 100"), "{}", c.evidence);
        // A second record of a counted campaign adds no hours, but its open crash is read, in
        // either order (verifier r1 F1: the second record hid its crash, a false PASS).
        let clean = fuzz_json("fuzzilli", &head, 100.0, true, "");
        let late = fuzz_json(
            "fuzzilli",
            &head,
            100.0,
            true,
            r#"{"id":"z9","state":"open"}"#,
        );
        for (first, second) in [(&clean, &late), (&late, &clean)] {
            o.fuzz = vec![
                input(&root, "a.json", first),
                input(&root, "a2.json", second),
                input(&root, "cargo.json", &cargo),
            ];
            let c = &run(&o).unwrap().cells[4].0;
            assert_eq!(c.status, Status::Fail, "{c:?}");
            assert!(
                c.evidence.contains("1 open unique crashes"),
                "{}",
                c.evidence
            );
            assert!(c.evidence.contains("z9 (fuzzilli, open)"), "{}", c.evidence);
            assert!(
                c.evidence.contains("Fuzzilli 100.0 / 100"),
                "{}",
                c.evidence
            );
        }
        // One crash id in two campaigns is one unique crash (verifier r1 F5).
        let x = r#"{"id":"x1","state":"open"}"#;
        o.fuzz = vec![
            input(
                &root,
                "a.json",
                &fuzz_json("fuzzilli", &head, 50.0, true, x),
            ),
            input(
                &root,
                "a2.json",
                &fuzz_json("fuzzilli", &head, 50.0, true, x)
                    .replace("fuzzilli-50", "fuzzilli-50-b"),
            ),
            input(&root, "cargo.json", &cargo),
        ];
        let c = &run(&o).unwrap().cells[4].0;
        assert_eq!(c.status, Status::Fail, "{c:?}");
        assert!(
            c.evidence.contains("Fuzzilli 100.0 / 100"),
            "{}",
            c.evidence
        );
        assert!(
            c.evidence.contains(", 1 open unique crashes"),
            "{}",
            c.evidence
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// F4 (plan v10 section 3.7; in:190 item 5) on the records of the producer,
    /// fuzz/scripts/record.sh (the fixtures `fuzz-*.json`, made by `make-fuzz-records.sh`): a
    /// campaign id counts once, and only from the record that counts. A record of the id that is
    /// not counted (dirty, or stale) does not block a later one that counts.
    #[test]
    fn item5_counts_a_campaign_from_the_record_that_counts() {
        let root = fake_repo("item5f4");
        let head = git_in(&root, &["rev-parse", "HEAD"]);
        let placeholder = "f4".repeat(20);
        let rec = |name: &str, commit: &str| {
            let text = read(&fixture(name)).unwrap();
            assert!(text.contains(&placeholder), "{name}: {text}");
            input(&root, name, &text.replace(&placeholder, commit))
        };
        let clean = rec("fuzz-fuzzilli.json", &head);
        let dirty = rec("fuzz-fuzzilli-dirty.json", &head);
        let cargo = rec("fuzz-cargo-fuzz.json", &head);
        let mut o = opts(&root);
        // The producer's records count: 100 + 24 CPU-h, PASS.
        o.fuzz = vec![clean.clone(), cargo.clone()];
        let c = &run(&o).unwrap().cells[4].0;
        assert_eq!(c.status, Status::Pass, "{c:?}");
        assert!(
            c.evidence.contains(
                "Fuzzilli 100.0 / 100 CPU-h, cargo-fuzz 24.0 / 24 CPU-h, 0 open unique crashes"
            ),
            "{}",
            c.evidence
        );
        // A dirty record of the campaign first, then the clean one: the clean one counts (the
        // id was inserted before the checks: 0 h, OPEN).
        o.fuzz = vec![dirty.clone(), clean.clone(), cargo.clone()];
        let c = &run(&o).unwrap().cells[4].0;
        assert!(
            c.evidence
                .contains("fuzz-fuzzilli-dirty.json: not counted: made with uncommitted changes"),
            "{}",
            c.evidence
        );
        assert!(!c.evidence.contains("again"), "{}", c.evidence);
        assert!(
            c.evidence.contains("Fuzzilli 100.0 / 100 CPU-h"),
            "{}",
            c.evidence
        );
        assert_eq!(
            c.status,
            Status::Open,
            "the dirty record is still named: {c:?}"
        );
        // The other order: the clean one counts, the dirty one is the same campaign again.
        o.fuzz = vec![clean.clone(), dirty, cargo.clone()];
        let c = &run(&o).unwrap().cells[4].0;
        assert_eq!(c.status, Status::Pass, "{c:?}");
        assert!(
            c.evidence.contains(
                "fuzz-fuzzilli-dirty.json: campaign fuzzilli \"fz-fixture-1\" again, its hours counted once"
            ),
            "{}",
            c.evidence
        );
        assert!(
            c.evidence.contains("Fuzzilli 100.0 / 100 CPU-h"),
            "{}",
            c.evidence
        );
        // A stale record of the campaign (a commit this repository does not have) first: it is
        // not counted, and the fresh one still counts.
        let stale = input(
            &root,
            "stale.json",
            &read(&clean).unwrap().replace(&head, &"e".repeat(40)),
        );
        o.fuzz = vec![stale, clean, cargo];
        let c = &run(&o).unwrap().cells[4].0;
        assert_eq!(
            c.status,
            Status::Open,
            "the stale record is still named: {c:?}"
        );
        assert!(
            c.evidence
                .contains("stale.json: not counted: its commit `eeee"),
            "{}",
            c.evidence
        );
        assert!(!c.evidence.contains("again"), "{}", c.evidence);
        assert!(
            c.evidence.contains("Fuzzilli 100.0 / 100 CPU-h"),
            "{}",
            c.evidence
        );
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
                let id = KERNEL_IDS
                    .get(i)
                    .map_or_else(|| format!("k{i}"), |k| k.to_string());
                if i < ok {
                    format!(r#"{{"id":"{id}","status":"runs, output equal","ok":true}}"#)
                } else {
                    format!(r#"{{"id":"{id}","status":"fails","ok":false,"blocker":"RegExp"}}"#)
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
            r#"{{"schema":"ep9-kernels/v1","commit":"{}","tree":{{{}}},"dirty":[],"host":{{"target":"{target}"}},"ep9":{{"bin":"ep9","engine_build":"{GATE_LINE}"}},"kernels":[{}]}}"#,
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
                "not ok: octane/navier-stokes (fails, blocker RegExp)",
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
            r#"{{"schema":"ep9-exit2/v1","commit":"{commit}","dirty":false,"engine_build":"{GATE_LINE}","host":{{"gate_host":{gate_host}}},"provisional":false,"readings":[{{"id":"octane7-vs-qjs","label":"Octane-7","gate":"{}","net":{{"median":1.1}}}},{{"id":"v8v7-7-vs-node-jitless","label":"v8-v7","gate":"{}"}}]}}"#,
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
                exit2_json(&"0123abcd".repeat(5), ["pass", "pass"], true),
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

    /// CX5-C2 (Codex round 1): kernels.mjs takes `--commit A` while it reads the tree ids of the
    /// checkout (B). The named commit must be in this repository and its ids of the fresh paths
    /// must be the reported ones; else OPEN.
    #[test]
    fn item1_the_named_commit_has_the_reported_tree_ids() {
        let root = fake_repo("item1commit");
        let a = git_in(&root, &["rev-parse", "HEAD"]);
        write(&root, "crates/ep9_vm/src/lib.rs", "// changed\n");
        let b = commit_all(&root, "vm");
        let mut o = opts(&root);
        // The ids of B (HEAD) with the commit A: crates differs between them.
        let j = kernels_json(&root, HOST, 15, 15, None);
        o.kernels = Some(input(&root, "kernels.json", &j));
        assert_eq!(run(&o).unwrap().cells[0].0.status, Status::Pass);
        let at_a = j.replace(&b, &a);
        o.kernels = Some(input(&root, "kernels.json", &at_a));
        let c = &run(&o).unwrap().cells[0].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence.contains(&format!(
                "its commit `{}` has other ids of crates than the reported ones",
                short(&a)
            )),
            "{}",
            c.evidence
        );
        // A full id that is no commit here.
        let gone = j.replace(&b, &"0123abcd".repeat(5));
        o.kernels = Some(input(&root, "kernels.json", &gone));
        let c = &run(&o).unwrap().cells[0].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence.contains(&format!(
                "its commit `{}` is not in this repository",
                short(&"0123abcd".repeat(5))
            )),
            "{}",
            c.evidence
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// CX5-C3 (Codex round 1): each expected kernel (item 1) and reading (item 2) once; a
    /// duplicate row does not stand in for a missing one.
    #[test]
    fn items_1_and_2_need_each_expected_id_once() {
        let root = fake_repo("ids");
        let head = git_in(&root, &["rev-parse", "HEAD"]);
        let mut o = opts(&root);
        // 15 rows, all of one passing kernel.
        let j = kernels_json(&root, HOST, 15, 15, None);
        let mut one = j.clone();
        for k in &KERNEL_IDS[1..] {
            one = one.replace(&format!(r#""id":"{k}""#), r#""id":"v8v7/richards""#);
        }
        o.kernels = Some(input(&root, "kernels.json", &one));
        let c = &run(&o).unwrap().cells[0].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence.contains("kernel v8v7/richards 15 times"),
            "{}",
            c.evidence
        );
        assert!(
            c.evidence.contains("missing kernels: v8v7/deltablue,"),
            "{}",
            c.evidence
        );
        // An id that is not one of the 15.
        let other = j.replace(r#""id":"octane/splay""#, r#""id":"octane/typescript""#);
        o.kernels = Some(input(&root, "kernels.json", &other));
        let c = &run(&o).unwrap().cells[0].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence.contains("missing kernels: octane/splay"),
            "{}",
            c.evidence
        );
        assert!(
            c.evidence.contains("unknown kernels: octane/typescript"),
            "{}",
            c.evidence
        );
        // Item 2: two copies of one passing reading.
        let two = exit2_json(&head, ["pass", "pass"], true)
            .replace("v8v7-7-vs-node-jitless", "octane7-vs-qjs");
        o.cross = Some(input(&root, "exit2.json", &two));
        let c = &run(&o).unwrap().cells[1].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence.contains("reading octane7-vs-qjs 2 times"),
            "{}",
            c.evidence
        );
        assert!(
            c.evidence
                .contains("missing readings: v8v7-7-vs-node-jitless"),
            "{}",
            c.evidence
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// CX5-C5 (Codex round 1): a measured tree with uncommitted changes proves nothing. Item 2:
    /// `dirty: true` or no `dirty` (run.mjs and exit2.mjs write it) gives OPEN. Item 5 (CX6-1,
    /// Codex round 2): only a record with `dirty: false` is counted; `dirty: true`, no `dirty`
    /// or a `dirty` that is not a JSON bool gives OPEN and adds no hours.
    #[test]
    fn items_2_and_5_read_the_dirty_flag() {
        let root = fake_repo("dirty");
        let head = git_in(&root, &["rev-parse", "HEAD"]);
        let mut o = opts(&root);
        let clean = exit2_json(&head, ["pass", "pass"], true);
        for (j, want, text) in [
            (clean.clone(), Status::Pass, "this tree (HEAD)"),
            (
                clean.replace(
                    r#""dirty":false"#,
                    r#""dirty":true,"dirty_paths":["crates/x.rs"]"#,
                ),
                Status::Open,
                "measured with uncommitted changes (dirty): crates/x.rs",
            ),
            (
                clean.replace(r#""dirty":false,"#, ""),
                Status::Open,
                "it does not say whether the tree was dirty",
            ),
        ] {
            o.cross = Some(input(&root, "exit2.json", &j));
            let c = &run(&o).unwrap().cells[1].0;
            assert_eq!(c.status, want, "{text}: {c:?}");
            assert!(c.evidence.contains(text), "{text}: {}", c.evidence);
        }
        let fuzz = |o: &mut Options, a: String| {
            o.fuzz = vec![
                input(&root, "f1.json", &a),
                input(
                    &root,
                    "f2.json",
                    &fuzz_json("cargo-fuzz", &head, 24.0, true, ""),
                ),
            ];
        };
        let f = fuzz_json("fuzzilli", &head, 100.0, true, "");
        fuzz(&mut o, f.replace(r#""dirty":false"#, r#""dirty":true"#));
        let c = &run(&o).unwrap().cells[4].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence
                .contains("f1.json: not counted: made with uncommitted changes (dirty)"),
            "{}",
            c.evidence
        );
        assert!(c.evidence.contains("Fuzzilli 0.0 / 100"), "{}", c.evidence);
        for bad in [
            f.replace(r#""dirty":false,"#, ""),
            f.replace(r#""dirty":false"#, r#""dirty":"false""#),
            f.replace(r#""dirty":false"#, r#""dirty":0"#),
        ] {
            fuzz(&mut o, bad);
            let c = &run(&o).unwrap().cells[4].0;
            assert_eq!(c.status, Status::Open, "{c:?}");
            assert!(
                c.evidence.contains(
                    "f1.json: not counted: it does not say whether its tree was dirty (no `dirty` bool)"
                ),
                "{}",
                c.evidence
            );
            assert!(c.evidence.contains("Fuzzilli 0.0 / 100"), "{}", c.evidence);
            assert!(!c.evidence.contains("NOTE: "), "{}", c.evidence);
        }
        fuzz(&mut o, f.clone());
        assert_eq!(run(&o).unwrap().cells[4].0.status, Status::Pass);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn gc_json(fails: &[&str], extra: &str, src: &str) -> String {
        let total = t262_files().len() as u64 - 1;
        format!(
            r#"{{"command":"exec","target":"{HOST}","gc_stress":1,"timeout_ms":60000,"feedback_threshold":0,{extra}{src}"skipped":1,"skipped_paths":["test/language/l19.js"],"files":{{"passed":{},"total":{total}}},"results":[{}],"expectations":{{"file":"x"}}}}"#,
            total - fails.len() as u64,
            results(fails)
        )
    }

    const S1: &str = r#""generational":true,"verify_cadence":1,"#;

    #[test]
    fn item4_same_pass_set_with_s1() {
        let root = fake_repo("item4");
        let src = src_at(&root, "HEAD");
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
                &src,
            ),
        ));
        for (j, want, text) in [
            (
                gc_json(&["test/language/l00.js"], S1, &src),
                Status::Pass,
                "S1 on, verify cadence 1, files 22 / 23 + 1 skipped, infra 0, new failures 0",
            ),
            (
                gc_json(&["test/language/l05.js"], S1, &src),
                Status::Fail,
                "pass without GC-stress, fail under it: test/language/l05.js",
            ),
            (
                gc_json(&[], "", &src),
                Status::Open,
                "no \"generational\" field",
            ),
            (
                gc_json(&[], r#""generational":true,"verify_cadence":8,"#, &src),
                Status::Fail,
                "verify cadence 8, not 1",
            ),
            (
                gc_json(&[], r#""generational":false,"verify_cadence":1,"#, &src),
                Status::Fail,
                "S1 off",
            ),
            (
                gc_json(&[], S1, &src).replace(r#""timeout_ms":60000"#, r#""timeout_ms":90000"#),
                Status::Fail,
                "timeout_ms 90000",
            ),
            (
                gc_json(&[], S1, &src)
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
        let named = gc_json(&["test/language/l00.js"], S1, &src).replace(
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

    /// X3, the O8 refusal (plan v10 section 3.7, V10-D10): items 1-4 read the build of each
    /// input. A missing field, a preset or feedback other than the gate's (`jit`, `full`), or
    /// features without `jit` give OPEN, never FAIL; the gate build passes.
    #[test]
    fn items_1_to_4_refuse_another_build() {
        let root = fake_repo("x3");
        let head = git_in(&root, &["rev-parse", "HEAD"]);
        let mut o = opts(&root);
        let gate = GATE_BUILD;
        // Items 3 and 4: the fields of an ep9-test262 JSON.
        let t262_cases: Vec<(String, Status, &str)> = vec![
            (
                gate.to_string(),
                Status::Pass,
                "build: preset jit, feedback full, features jit,become",
            ),
            (
                gate.replace(r#""preset":"jit""#, r#""preset":"jitless""#),
                Status::Open,
                "build: preset jitless, the gate reads preset jit (V10-D10)",
            ),
            (
                gate.replace(r#""mode":"jit""#, r#""mode":"jitless""#),
                Status::Open,
                "build: its isolates ran preset jitless (`mode`), the gate reads preset jit",
            ),
            (
                gate.replace(r#""feedback":"full""#, r#""feedback":"ics_only""#),
                Status::Open,
                "build: feedback ics_only, the gate reads feedback full (V10-D10)",
            ),
            (
                gate.replace(r#"["jit","become"]"#, r#"["become"]"#),
                Status::Open,
                "build: features become, the gate reads a build with jit (V10-D10)",
            ),
            (
                gate.replace(r#"["jit","become"]"#, "[]"),
                Status::Open,
                "build: features none, the gate reads a build with jit",
            ),
            // A JSON from before M5-P0: `mode` only.
            (
                r#""mode":"jit","#.to_string(),
                Status::Open,
                "build: it names no preset (`preset`), the gate reads preset jit",
            ),
            (
                gate.replace(r#""feedback":"full","#, ""),
                Status::Open,
                "build: it names no feedback (`feedback`), the gate reads feedback full",
            ),
            (
                gate.replace(r#""features":["jit","become"],"#, ""),
                Status::Open,
                "build: it names no features (`features`), the gate reads a build with jit",
            ),
            // A non-string feature is no feature list.
            (
                gate.replace(r#"["jit","become"]"#, r#"["jit",1]"#),
                Status::Open,
                "build: it names no features (`features`)",
            ),
        ];
        let bare = src_at(&root, "HEAD").replace(GATE_BUILD, "");
        for (build, want, text) in &t262_cases {
            let src = format!("{bare}{build}");
            o.exec = Some(input(
                &root,
                "exec.json",
                &exec_json(17, 20, 24, 0.0, &[], &src),
            ));
            o.exec_gc = Vec::new();
            let c = &run(&o).unwrap().cells[2].0;
            assert_eq!(c.status, *want, "item 3, {text}: {c:?}");
            assert!(c.evidence.contains(text), "item 3, {text}: {}", c.evidence);
            // Item 4: the GC-stress JSON (the --exec run of the gate build).
            o.exec = Some(input(
                &root,
                "exec.json",
                &exec_json(17, 20, 24, 0.0, &[], &src_at(&root, "HEAD")),
            ));
            o.exec_gc = vec![input(&root, "gc.json", &gc_json(&[], S1, &src))];
            let c = &run(&o).unwrap().cells[3].0;
            assert_eq!(c.status, *want, "item 4, {text}: {c:?}");
            assert!(c.evidence.contains(text), "item 4, {text}: {}", c.evidence);
        }
        // Item 4 also reads the build of the --exec run it compares with.
        let src = src_at(&root, "HEAD");
        o.exec = Some(input(
            &root,
            "exec.json",
            &exec_json(
                17,
                20,
                24,
                0.0,
                &[],
                &src.replace(r#""feedback":"full""#, r#""feedback":"ics_only""#),
            ),
        ));
        o.exec_gc = vec![input(&root, "gc.json", &gc_json(&[], S1, &src))];
        let c = &run(&o).unwrap().cells[3].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence
                .contains("exec.json: build: feedback ics_only, the gate reads feedback full"),
            "{}",
            c.evidence
        );
        // Items 1 and 2: the `--version` build line the bench JSONs copy.
        let line_cases: [(Option<&str>, Status, &str); 6] = [
            (
                Some(GATE_LINE),
                Status::Pass,
                "build: preset jit, feedback full (the preset's default), features jit,become",
            ),
            (
                Some("features jit; preset jit; feedback full"),
                Status::Pass,
                "build: preset jit, feedback full, features jit",
            ),
            (
                Some("features none; preset jitless"),
                Status::Open,
                "build: preset jitless, the gate reads preset jit (V10-D10); build: feedback ics_only, the gate reads feedback full (V10-D10); build: features none, the gate reads a build with jit",
            ),
            (
                Some("features jit; preset jit; feedback ics_only"),
                Status::Open,
                "build: feedback ics_only, the gate reads feedback full",
            ),
            // A binary from before M5-P0 prints no build line: kernels.mjs writes null.
            (
                None,
                Status::Open,
                "build: it names no preset (`preset`), the gate reads preset jit",
            ),
            (
                Some("unknown (--version: exit 1)"),
                Status::Open,
                "build: it names no features (`features`)",
            ),
        ];
        for (line, want, text) in line_cases {
            let value = line.map_or("null".to_string(), |l| format!("\"{l}\""));
            let k = kernels_json(&root, HOST, 15, 15, None)
                .replace(&format!("\"{GATE_LINE}\""), &value);
            o.kernels = Some(input(&root, "kernels.json", &k));
            let c = &run(&o).unwrap().cells[0].0;
            assert_eq!(c.status, want, "item 1, {text}: {c:?}");
            assert!(c.evidence.contains(text), "item 1, {text}: {}", c.evidence);
            let e = exit2_json(&head, ["pass", "pass"], true)
                .replace(&format!("\"{GATE_LINE}\""), &value);
            o.cross = Some(input(&root, "exit2.json", &e));
            let c = &run(&o).unwrap().cells[1].0;
            assert_eq!(c.status, want, "item 2, {text}: {c:?}");
            assert!(c.evidence.contains(text), "item 2, {text}: {}", c.evidence);
        }
        // An exit2 JSON with no `engine_build` field (exit2.mjs of M5-P0 copies none): OPEN.
        let e = exit2_json(&head, ["pass", "pass"], true)
            .replace(&format!(r#""engine_build":"{GATE_LINE}","#), "");
        o.cross = Some(input(&root, "exit2.json", &e));
        let c = &run(&o).unwrap().cells[1].0;
        assert_eq!(c.status, Status::Open, "{c:?}");
        assert!(
            c.evidence.contains("build: it names no preset (`preset`)"),
            "{}",
            c.evidence
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn fuzz_json(tool: &str, commit: &str, hours: f64, counted: bool, crash: &str) -> String {
        format!(
            r#"{{"schema":"ep9-fuzz/v1","tool":"{tool}","campaign":"{tool}-{hours}","target":"t","commit":"{commit}","dirty":false,"cpu_hours":{hours},"counted":{counted},"crashes":[{crash}]}}"#
        )
    }

    /// M4-P5 C3: items 1, 2 and 5 take a full commit id only. `"HEAD"` resolves to whatever the
    /// tree is now, and a short or upper-case id is not the form the tools write: each gives OPEN.
    #[test]
    fn items_1_2_and_5_need_a_full_commit_id() {
        let root = fake_repo("fullsha");
        let head = git_in(&root, &["rev-parse", "HEAD"]);
        let mut o = opts(&root);
        let kernels = kernels_json(&root, HOST, 15, 15, None);
        let set = |o: &mut Options, commit: &str| {
            o.kernels = Some(input(
                &root,
                "kernels.json",
                &kernels.replace(
                    &format!(r#""commit":"{head}""#),
                    &format!(r#""commit":"{commit}""#),
                ),
            ));
            o.cross = Some(input(
                &root,
                "exit2.json",
                &exit2_json(commit, ["pass", "pass"], true),
            ));
            o.fuzz = vec![
                input(
                    &root,
                    "f1.json",
                    &fuzz_json("fuzzilli", commit, 100.0, true, ""),
                ),
                input(
                    &root,
                    "f2.json",
                    &fuzz_json("cargo-fuzz", &head, 24.0, true, ""),
                ),
            ];
        };
        set(&mut o, &head);
        let r = run(&o).unwrap();
        for i in [0, 1, 4] {
            assert_eq!(
                r.cells[i].0.status,
                Status::Pass,
                "item {}: {:?}",
                i + 1,
                r.cells[i].0
            );
        }
        for bad in [
            "HEAD".to_string(),
            head[..7].to_string(),
            head.to_uppercase(),
        ] {
            set(&mut o, &bad);
            let r = run(&o).unwrap();
            for i in [0, 1, 4] {
                let c = &r.cells[i].0;
                assert_eq!(c.status, Status::Open, "item {} {bad}: {c:?}", i + 1);
                assert!(
                    c.evidence.contains(&format!(
                        "its commit {bad:?} is not a commit id (40 hex digits)"
                    )),
                    "item {} {bad}: {}",
                    i + 1,
                    c.evidence
                );
            }
        }
        std::fs::remove_dir_all(root).unwrap();
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
                "not counted: not this tree: crates, Cargo.toml, Cargo.lock, rust-toolchain.toml, third_party/regress.commit, third_party/patches/regress changed"
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

    /// fuzzrec high 1: a record cut off before its summary (counted false, fuzz.yml's fallback)
    /// still carries its open crash ids, and item 5 reads them: FAIL, even with enough counted hours.
    #[test]
    fn item5_reads_the_crashes_of_an_uncounted_record() {
        let root = fake_repo("item5unc");
        let head = git_in(&root, &["rev-parse", "HEAD"]);
        let mut o = opts(&root);
        let cut = fuzz_json(
            "cargo-fuzz",
            &head,
            0.0,
            false,
            r#"{"id":"c3","state":"open"}"#,
        )
        .replace(r#""campaign":"cargo-fuzz-0""#, r#""campaign":"gh1-1-t""#);
        o.fuzz = vec![
            input(
                &root,
                "f1.json",
                &fuzz_json("fuzzilli", &head, 100.0, true, ""),
            ),
            input(
                &root,
                "f2.json",
                &fuzz_json("cargo-fuzz", &head, 24.0, true, ""),
            ),
            input(&root, "f3.json", &cut),
        ];
        let c = &run(&o).unwrap().cells[4].0;
        assert_eq!(c.status, Status::Fail, "{c:?}");
        assert!(
            c.evidence.contains("1 open unique crashes"),
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

    /// gates5 medium 5: the evidence shortening cuts on character boundaries, so an input field
    /// with non-ASCII text cannot panic m1-exit; an ASCII id is cut as before.
    #[test]
    fn short_is_safe_for_any_text() {
        let id = "0123abcd".repeat(5);
        assert_eq!(short(&id), "0123abcd\u{2026}3abcd");
        assert_eq!(short("abc"), "abc");
        let e = "\u{e9}".repeat(20);
        assert_eq!(
            short(&e),
            format!("{}\u{2026}{}", "\u{e9}".repeat(8), "\u{e9}".repeat(5))
        );
        assert_eq!(short(&"\u{e9}".repeat(12)), "\u{e9}".repeat(12));
    }
}
