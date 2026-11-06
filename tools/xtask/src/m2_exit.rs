//! `cargo xtask m2-exit` (M2 exit check, docs/plan/09-milestones.md section 2.2, M2 exit 1-10;
//! plan v12 GATES10 task 3): the table of the 10 M2 exit items and the D11 row, PASS, FAIL, OPEN,
//! N/A or NOT-RUN per item, for one host. Run it once per host: each run is one column.
//!
//! ```text
//! cargo xtask m2-exit --host <triple> --out m2-exit.md [--root <dir>]
//!     [--gate <json>]         tools/bench/cross/m2gate.mjs --out (ep9-m2gate/v1)   items 1, 2, 3, 6, 10, D11
//!     [--linux-gate <json>]   the m2gate JSON of the Linux x64 host (Windows host only: the ratio)
//!     [--mem <json>]          tools/bench/cross/mem.mjs --out (ep9-mem/v1)          item 4
//!     [--exec <json>]         ep9-test262 exec --json (the CI artifact)             item 5
//!     [--fastdiff <json>]     the FASTDIFF JSON (ep9-fastdiff/v1)                    item 5
//!     [--jetstream <json>]    tools/bench/jetstream/run.mjs --out                    item 7
//!     [--fuzz <json>]...      one ep9-fuzz/v1 record per counted campaign            item 8
//!     [--p-gates <md>]        docs/measurements/<date>-p-gates.md                    item 9
//! ```
//!
//! The task reads its inputs; it runs no benchmark and no test suite. A missing input gives
//! NOT-RUN, and the report prints one line per missing input with the command that makes it.
//! The tree root is `--root`, or the git top dir of the current dir (`exit::default_root`).
//!
//! | Item | Cell |
//! |---|---|
//! | 1, 2, 3, 6 | the m2gate readings `octane7-vs-qjs` (>= 1.33x QuickJS), `v8v7-7-vs-node-jitless` (geomean >= 1.0x node --jitless; a kernel below 0.8x is parity debt, 09 section 5), `ts` (V12-D5) and `splay-vs-node-jitless` (>= 1.0x): PASS only when the reading's `gate` is `pass` in **both** builds, the jit preset and the jitless build (V12-D6); a `fail` is FAIL; a marginal or provisional reading, or a missing build, is OPEN |
//! | 4 | MEMREAD: per build and v8-v7 kernel, the median peak RSS of ep9 over node --jitless <= 1.2 and over d8 --jitless <= 1.8 (G7, 02-architecture.md), >= 5 runs per row (class Mem), the load guard passed |
//! | 5 | the exec JSON by directory, by the G1 file rule (a file passes when its result is a pass; a skipped file fails): `language/` >= 95%, `annexB/` >= 90%, `built-ins/{Array, String, Object, Function, Math, JSON, Number, RegExp}` >= 95% each; and the FASTDIFF JSON: 0 mismatches, every fast path with >= 1 case (0 cases is OPEN) |
//! | 7 | the JetStream run: every workload of `expectations/jetstream-m2.txt` has all its repetitions on each ep9 config and on node --jitless; the ratio is printed |
//! | 8 | Fuzzilli >= 500 counted CPU-h, 0 open unique crashes (the m1-exit record rules) |
//! | 9 | the P-gate file: one row each for P1, P2 and P4 with the result `not tripped` or `tripped` |
//! | 10 | the jit preset readings of items 1-3 pass (feedback full is the jit preset default, 09:113) |
//! | D11 | the jitless build readings of items 1-3 pass (11-decisions.md D11) |
//!
//! **Hosts.** `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu` hold the speed gates
//! (09-milestones.md section 2.2). A Linux x64 speed cell reads OPEN "marginal (no gate host)"
//! when a run is not from a gate host (10-quality.md section 3, "Owner action"). On
//! `aarch64-unknown-linux-gnu` the speed cells are N/A by design (no gate host). On
//! `x86_64-pc-windows-msvc` a speed cell gives the ratio of each Windows reading to the Linux x64
//! one (`--linux-gate`; 09:66: within 10%; each reading on both hosts must have the status
//! `valid`, a gate verdict is not needed), and reads OPEN "marginal (no gate host)" until both
//! runs are gate-host runs on the same CPU model (a dual-boot or identical host, 10:157).
//!
//! **Fresh inputs.** A speed, MEMREAD, FASTDIFF or JetStream input names a full commit id that is
//! HEAD, or an ancestor with no change in the paths it measures since, and says that its tree
//! had no uncommitted change; else the cell is at least OPEN. An exec JSON may instead name the
//! git id of each path it measures (a CI pull_request job runs on GitHub's merge commit).
//!
//! **The build.** An m2gate run names its build (`jit` or `jitless`) and the build line of
//! `ep9 --version` (`features <list>; preset <name>`). The cell is OPEN when they disagree: the
//! jit preset build has preset `jit` and the feature `jit`; the jitless build has preset
//! `jitless` and no feature `jit`.
//!
//! **Input forms** (fixtures in `tests/m2_exit/`; the producers write these fields):
//!
//! - ep9-m2gate/v1 (m2gate.mjs): `builds[]` with `build`, `commit`, `dirty`, `engine_build`,
//!   `host.{id, gate_host, cpu, platform}`; `readings[]` with `id`, `build`, `gate`, `value`,
//!   `interval`, `reasons`, and for `v8v7-7-vs-node-jitless` the list `parity_debt`.
//! - ep9-mem/v1 (mem.mjs): `commit.{ep9, runner_dirty}`, `host.measured.platform`,
//!   `load.{before, after}.pass`, `engines[]` with `id` and `engine_build` (the `ep9 --version`
//!   build line), and `table[]` with `workload`, `engine` (an engine id: `ep9` is the jit preset
//!   build, `ep9-jitless` the jitless build, then `node-jitless`, `d8-jitless`), `n` (the runs)
//!   and `median_bytes` (the median peak RSS).
//! - ep9-fastdiff/v1 (the FASTDIFF test): `commit`, `dirty`, `fast_paths[]` with `name`, `cases`
//!   and `mismatches`.
//! - JetStream (jetstream/run.mjs): `list`, `jetstream_commit`, `reps`, `host.{platform, arch}`,
//!   `ep9.{commit, dirty}`, `configs[]` (`name`, `engine`, `kind`, `version`, `flags`) and
//!   `table[]` (per workload `<config>` median score and `<config>_n` scores). The ep9 configs are
//!   the ones of kind `ep9` (an `ep9` or `ep9@<path>` engine), and the version of each holds the
//!   build line of `ep9 --version`; node --jitless is the config of engine `node` (or kind
//!   `node`: a `node@<path>` engine) with the flag `--jitless`. Two configs of one name read
//!   OPEN (the table is keyed by name).
//! - P-gate file: a Markdown table with the columns `Gate` and `Result`.
//!
//! The extra rows (no exit-code effect): the parity debt of item 2 and the tripped P-gates.
//!
//! Exit 0 when every cell is PASS or N/A, 1 otherwise, 2 on a usage error or an input that
//! cannot be read.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::exit::cross::fresh_since;
use crate::exit::gc_shards::test262_files;
use crate::exit::json::{Json, parse_json};
use crate::exit::report::{Cell, Status, cell, not_given, short, some_paths};
use crate::exit::t262::{count, exec_health, pct};
use crate::exit::{git, read};

/// The 10 M2 exit items (09-milestones section 2.2, M2 exit) and the D11 row, short titles.
const TITLES: [&str; 11] = [
    "Octane-7 >= 1.33x QuickJS",
    "v8-v7 7-kernel geomean >= 1.0x node --jitless (each >= 0.8x or parity debt)",
    "TS workload <= 2.8 s",
    "Both G7 memory caps hold",
    "test262 dirs (language/, annexB/, 8 built-ins dirs); fast paths: 0 mismatches",
    "Splay >= 1.0x node --jitless",
    "The JetStream 3 M2 list runs; ratio to node --jitless recorded",
    "Fuzzilli 500 CPU-h, 0 unique crashes",
    "P1, P2 and P4 checked",
    "Gates 1-3 hold with all H16/H29 profiling on (jit preset)",
    "D11: gates 1-3 also in the jitless build",
];

/// The row labels of the table: items 1-10, then D11.
const LABELS: [&str; 11] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "D11"];

/// The m2gate reading ids of items 1, 2, 3 and 6.
const OCTANE: &str = "octane7-vs-qjs";
const V8V7: &str = "v8v7-7-vs-node-jitless";
const TS: &str = "ts";
const SPLAY: &str = "splay-vs-node-jitless";

/// The two builds of every M2 speed reading (V12-D6).
const BUILDS: [&str; 2] = ["jit", "jitless"];

/// What a speed input measured: the engine, its build and the regress fork it links.
const FRESH_PATHS: [&str; 7] = [
    "crates",
    "tools/bench",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "third_party/regress.commit",
    "third_party/patches/regress",
];

/// What a FASTDIFF run tested: the engine (no bench tool).
const ENGINE_PATHS: [&str; 6] = [
    "crates",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "third_party/regress.commit",
    "third_party/patches/regress",
];

/// What a test262 run read (as m1-exit's fresh, named inputs).
const T262_PATHS: [&str; 9] = [
    "crates",
    "tools/bench",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "tools/test262",
    "third_party/test262.commit",
    "third_party/regress.commit",
    "third_party/patches/regress",
];

/// Item 5 dirs: (prefix under `test/`, the percent it needs).
const T262_DIRS: [(&str, u64); 10] = [
    ("language", 95),
    ("annexB", 90),
    ("built-ins/Array", 95),
    ("built-ins/String", 95),
    ("built-ins/Object", 95),
    ("built-ins/Function", 95),
    ("built-ins/Math", 95),
    ("built-ins/JSON", 95),
    ("built-ins/Number", 95),
    ("built-ins/RegExp", 95),
];

/// G7 (02-architecture.md): the median peak RSS of ep9 over each comparator, at most.
const MEM_CAPS: [(&str, f64); 2] = [("node-jitless", 1.2), ("d8-jitless", 1.8)];
/// Class Mem: at least 5 runs (10-quality.md section 3).
const MEM_RUNS: f64 = 5.0;
/// The 7 v8-v7 kernels of M2 exit 2 and G7 (no regexp).
const V8V7_7: [&str; 7] = [
    "richards",
    "deltablue",
    "crypto",
    "raytrace",
    "earley-boyer",
    "splay",
    "navier-stokes",
];

const JETSTREAM_LIST: &str = "expectations/jetstream-m2.txt";
const JETSTREAM_PIN: &str = "third_party/bench-jetstream.commit";

const FUZZILLI_HOURS: f64 = 500.0;
const CLOSED: [&str; 3] = ["fixed", "duplicate", "not-a-bug"];
const FUZZ_PATHS: [&str; 6] = ENGINE_PATHS;

const P_GATES: [&str; 3] = ["P1", "P2", "P4"];
const P_RESULTS: [&str; 2] = ["not tripped", "tripped"];

/// Windows x64 within 10% of Linux x64 (09:66).
const WINDOWS_MIN: f64 = 0.9;

const GATE_CMD: &str = "node tools/bench/cross/run.mjs --workloads m2 --harness base --host <host file> --ep9 <build> --out run-<build>.json, for the jit preset and the jitless build, then node tools/bench/cross/m2gate.mjs run-jit.json run-jitless.json --out m2gate.json (on a gate host)";
const MEM_CMD: &str = "node tools/bench/cross/mem.mjs --out mem.json (BENCH10)";
const EXEC_CMD: &str = "cargo run --release --locked -p ep9_test262 -- exec --json exec.json (or the test262-exec.json artifact of the CI test262 job of this host)";
const FASTDIFF_CMD: &str =
    "cargo test --locked -p ep9_builtins --test fastpath_diff (its ep9-fastdiff/v1 JSON)";
const JETSTREAM_CMD: &str = "node tools/bench/jetstream/run.mjs --list expectations/jetstream-m2.txt --config ep9=ep9@<ep9 binary> --config jitless=node:--jitless,cli:--no-prefetch --out jetstream.json";
const FUZZ_HOW: &str = "one ep9-fuzz/v1 record per counted Fuzzilli campaign (the month-end batch)";
const P_GATES_HOW: &str = "docs/measurements/<date>-p-gates.md: a table with Gate and Result columns, one row each for P1, P2 and P4 (Result: not tripped or tripped)";

/// The 4 tier-1 targets and the node platform name (`<platform>-<arch>`) of each.
const TRIPLE_PLATFORMS: [(&str, &str); 4] = [
    ("aarch64-apple-darwin", "darwin-arm64"),
    ("x86_64-unknown-linux-gnu", "linux-x64"),
    ("aarch64-unknown-linux-gnu", "linux-arm64"),
    ("x86_64-pc-windows-msvc", "win32-x64"),
];
const LINUX_X64: &str = "x86_64-unknown-linux-gnu";
const LINUX_A64: &str = "aarch64-unknown-linux-gnu";
const WINDOWS_X64: &str = "x86_64-pc-windows-msvc";

const USAGE: &str = "usage: cargo xtask m2-exit --host <triple> --out <file.md> [--root <dir>] \
     [--gate <json>] [--linux-gate <json>] [--mem <json>] [--exec <json>] [--fastdiff <json>] \
     [--jetstream <json>] [--fuzz <json>]... [--p-gates <md>]";

/// The parsed command line.
#[derive(Debug, Default)]
struct Options {
    root: PathBuf,
    host: String,
    out: PathBuf,
    gate: Option<PathBuf>,
    linux_gate: Option<PathBuf>,
    mem: Option<PathBuf>,
    exec: Option<PathBuf>,
    fastdiff: Option<PathBuf>,
    jetstream: Option<PathBuf>,
    fuzz: Vec<PathBuf>,
    p_gates: Option<PathBuf>,
}

fn parse_args(args: &[String], root: PathBuf) -> Result<Options, String> {
    let mut o = Options {
        root,
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
            "--gate" => o.gate = Some(PathBuf::from(val()?)),
            "--linux-gate" => o.linux_gate = Some(PathBuf::from(val()?)),
            "--mem" => o.mem = Some(PathBuf::from(val()?)),
            "--exec" => o.exec = Some(PathBuf::from(val()?)),
            "--fastdiff" => o.fastdiff = Some(PathBuf::from(val()?)),
            "--jetstream" => o.jetstream = Some(PathBuf::from(val()?)),
            "--fuzz" => o.fuzz.push(PathBuf::from(val()?)),
            "--p-gates" => o.p_gates = Some(PathBuf::from(val()?)),
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }
    if o.host.is_empty() || o.out.as_os_str().is_empty() {
        return Err(format!("--host and --out are required\n{USAGE}"));
    }
    if platform_of(&o.host).is_none() {
        return Err(format!(
            "--host {:?} is not a tier-1 target ({})",
            o.host,
            TRIPLE_PLATFORMS
                .iter()
                .map(|(t, _)| *t)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if o.linux_gate.is_some() && o.host != WINDOWS_X64 {
        return Err(format!(
            "--linux-gate is for --host {WINDOWS_X64} only\n{USAGE}"
        ));
    }
    Ok(o)
}

/// Entry point: writes the report to `--out` and prints the table and the missing inputs.
pub fn main(args: &[String]) -> Result<bool, String> {
    let default = crate::exit::default_root();
    let opts = parse_args(args, default.clone().unwrap_or_default())?;
    if opts.root.as_os_str().is_empty() {
        return Err(default
            .err()
            .unwrap_or_else(|| format!("--root needs a dir\n{USAGE}")));
    }
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
    println!("m2-exit: wrote {}", opts.out.display());
    Ok(report.cells.iter().all(|c| c.status.ok()))
}

/// The rendered result.
#[derive(Debug)]
struct Report {
    table: String,
    markdown: String,
    /// Items 1-10, then D11.
    cells: Vec<Cell>,
    /// The extra rows: (what, cell). No exit-code effect.
    extra: Vec<(&'static str, Cell)>,
    /// One line per missing input: the flag, its items and the command that makes it.
    missing: Vec<String>,
}

/// Everything the items read, loaded once.
struct Ctx<'a> {
    o: &'a Options,
    gate: Option<(PathBuf, Json)>,
    linux_gate: Option<(PathBuf, Json)>,
    mem: Option<(PathBuf, Json)>,
    exec: Option<(PathBuf, Json)>,
    fastdiff: Option<(PathBuf, Json)>,
    jetstream: Option<(PathBuf, Json)>,
    fuzz: Vec<(PathBuf, Json)>,
    p_gates: Option<(PathBuf, String)>,
}

fn read_json(p: &Path) -> Result<(PathBuf, Json), String> {
    Ok((
        p.to_path_buf(),
        parse_json(&read(p)?).map_err(|e| format!("{}: {e}", p.display()))?,
    ))
}

/// The input's `schema` must be `want` (a usage error otherwise: the wrong file was given).
fn schema(flag: &str, input: &Option<(PathBuf, Json)>, want: &str) -> Result<(), String> {
    match input {
        Some((p, j)) if j.get("schema").and_then(Json::str) != Some(want) => Err(format!(
            "{flag} {}: not an {want} file (schema {:?})",
            p.display(),
            j.get("schema").and_then(Json::str).unwrap_or("none")
        )),
        _ => Ok(()),
    }
}

fn run(o: &Options) -> Result<Report, String> {
    let one = |p: &Option<PathBuf>| p.as_deref().map(read_json).transpose();
    let c = Ctx {
        o,
        gate: one(&o.gate)?,
        linux_gate: one(&o.linux_gate)?,
        mem: one(&o.mem)?,
        exec: one(&o.exec)?,
        fastdiff: one(&o.fastdiff)?,
        jetstream: one(&o.jetstream)?,
        fuzz: o
            .fuzz
            .iter()
            .map(|p| read_json(p))
            .collect::<Result<_, _>>()?,
        p_gates: o
            .p_gates
            .as_ref()
            .map(|p| Ok::<_, String>((p.clone(), read(p)?)))
            .transpose()?,
    };
    schema("--gate", &c.gate, "ep9-m2gate/v1")?;
    schema("--linux-gate", &c.linux_gate, "ep9-m2gate/v1")?;
    schema("--mem", &c.mem, "ep9-mem/v1")?;
    schema("--fastdiff", &c.fastdiff, "ep9-fastdiff/v1")?;
    if let Some((p, j)) = &c.exec {
        if j.get("command").and_then(Json::str) != Some("exec") {
            return Err(format!(
                "--exec {}: not an ep9-test262 exec JSON",
                p.display()
            ));
        }
        let t = j.get("target").and_then(Json::str).unwrap_or("");
        if t != o.host {
            return Err(format!(
                "--exec {}: target {t:?}, but --host is {:?}",
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
    // No gate host on Linux a64: its speed cells need no input.
    miss(
        c.gate.is_some() || o.host == LINUX_A64,
        "--gate",
        "items 1, 2, 3, 6, 10, D11",
        GATE_CMD,
    );
    if o.host == WINDOWS_X64 {
        miss(
            c.linux_gate.is_some(),
            "--linux-gate",
            "the Windows ratio of items 1, 2, 3, 6, 10, D11",
            "the m2gate JSON of the Linux x64 run",
        );
    }
    miss(c.mem.is_some(), "--mem", "item 4", MEM_CMD);
    miss(c.exec.is_some(), "--exec", "item 5", EXEC_CMD);
    miss(c.fastdiff.is_some(), "--fastdiff", "item 5", FASTDIFF_CMD);
    miss(
        c.jetstream.is_some(),
        "--jetstream",
        "item 7",
        JETSTREAM_CMD,
    );
    miss(!c.fuzz.is_empty(), "--fuzz", "item 8", FUZZ_HOW);
    miss(c.p_gates.is_some(), "--p-gates", "item 9", P_GATES_HOW);
    let cells = vec![
        speed_cell(&c, &[OCTANE], &BUILDS),
        speed_cell(&c, &[V8V7], &BUILDS),
        speed_cell(&c, &[TS], &BUILDS),
        item4(&c),
        item5(&c),
        speed_cell(&c, &[SPLAY], &BUILDS),
        item7(&c),
        item8(&c),
        item9(&c),
        speed_cell(&c, &[OCTANE, V8V7, TS], &["jit"]),
        speed_cell(&c, &[OCTANE, V8V7, TS], &["jitless"]),
    ];
    let extra = vec![
        ("parity debt", debt_cell(&c)),
        ("P-gates tripped", trip_cell(&c)),
    ];
    Ok(render(&c, cells, extra, missing))
}

// ---- shared checks ------------------------------------------------------------------------

/// The node platform name (`<platform>-<arch>`) of a target triple.
fn platform_of(triple: &str) -> Option<&'static str> {
    TRIPLE_PLATFORMS
        .iter()
        .find(|(t, _)| *t == triple)
        .map(|(_, p)| *p)
}

/// A full commit id: 40 lower-case hex digits (a ref name names no fixed commit).
fn is_full_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `text` (`, reason; reason`) when there are reasons.
fn because(why: &[String]) -> String {
    if why.is_empty() {
        String::new()
    } else {
        format!("; {}", why.join("; "))
    }
}

/// The input measured this tree: `commit` is a full commit id, HEAD or an ancestor with no
/// change in `paths` since, and `dirty` is `false` (or an empty list). Err(why) gives OPEN.
fn fresh(
    root: &Path,
    commit: Option<&str>,
    dirty: Option<&Json>,
    paths: &[&str],
) -> Result<String, String> {
    let commit = commit.unwrap_or("");
    if commit.is_empty() {
        return Err("it names no ep9 commit".into());
    }
    if !is_full_sha(commit) {
        return Err(format!(
            "its commit {commit:?} is not a commit id (40 hex digits)"
        ));
    }
    match dirty {
        Some(Json::Bool(false)) => {}
        Some(Json::Arr(a)) if a.is_empty() => {}
        Some(Json::Bool(true)) => return Err("it was made with uncommitted changes (dirty)".into()),
        Some(Json::Arr(a)) => {
            return Err(format!(
                "it was made with uncommitted changes in {}",
                some_paths(
                    &a.iter()
                        .filter_map(Json::str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                )
            ));
        }
        _ => return Err("it does not say whether its tree was dirty (no `dirty` field)".into()),
    }
    fresh_since(root, commit, paths)
}

/// The git ids of `paths` at HEAD equal the input's `tree` ids (a CI job on a merge commit).
fn same_trees(root: &Path, tree: Option<&Json>, paths: &[&str]) -> bool {
    let Some(Json::Obj(_)) = tree else {
        return false;
    };
    paths.iter().all(|p| {
        let want = tree.and_then(|t| t.get(p)).and_then(Json::str);
        let got = git(
            root,
            &["rev-parse", "--verify", "--quiet", &format!("HEAD:{p}")],
        );
        want.is_some() && want.map(str::to_string) == got
    })
}

/// `cell` with its status raised to OPEN at least when the input is not fresh.
fn with_fresh(c: Cell, fresh: Result<String, String>) -> Cell {
    match fresh {
        Ok(t) => cell(c.status, format!("{}; fresh: {t}", c.evidence)),
        Err(why) => cell(
            c.status.max(Status::Open),
            format!("{}; stale input: {why}", c.evidence),
        ),
    }
}

/// Status of a platform check (X12 form): another platform is FAIL, none is OPEN.
fn platform_check(host: &str, got: Option<&str>, what: &str, why: &mut Vec<String>) -> Status {
    let want = platform_of(host).unwrap_or("");
    match got {
        Some(p) if p == want => Status::Pass,
        Some(p) => {
            why.push(format!(
                "{what} measured on {p}, not {want} (--host {host})"
            ));
            Status::Fail
        }
        None => {
            why.push(format!("{what} names no host platform"));
            Status::Open
        }
    }
}

// ---- items 1, 2, 3, 6, 10, D11: the m2gate readings --------------------------------------

/// The build block of `build` in an m2gate JSON.
fn build_block<'a>(j: &'a Json, build: &str) -> Option<&'a Json> {
    j.get("builds")?
        .arr()?
        .iter()
        .find(|b| b.get("build").and_then(Json::str) == Some(build))
}

/// The reading `id` of `build`.
fn reading<'a>(j: &'a Json, id: &str, build: &str) -> Option<&'a Json> {
    j.get("readings")?.arr()?.iter().find(|r| {
        r.get("id").and_then(Json::str) == Some(id)
            && r.get("build").and_then(Json::str) == Some(build)
    })
}

/// The build line `features <list>; preset <name>` names `build`: Err(why) otherwise.
fn build_line_check(build: &str, line: Option<&str>) -> Result<(), String> {
    let Some(line) = line else {
        return Err(format!(
            "the {build} run names no build line (engine_build)"
        ));
    };
    let (features, preset) = line
        .split_once(';')
        .map(|(f, p)| (f.trim(), p.trim()))
        .unwrap_or((line, ""));
    let features: BTreeSet<&str> = features
        .strip_prefix("features ")
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .collect();
    let preset = preset.strip_prefix("preset ").unwrap_or("");
    let ok = match build {
        "jit" => preset == "jit" && features.contains("jit"),
        "jitless" => preset == "jitless" && !features.contains("jit"),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "the {build} run's build line is {line:?} (the {build} build needs {})",
            if build == "jit" {
                "preset jit and the feature jit"
            } else {
                "preset jitless and no feature jit"
            }
        ))
    }
}

fn num(j: &Json, key: &str) -> Option<f64> {
    j.get(key).and_then(Json::num)
}

fn interval_text(r: &Json) -> String {
    match r.get("interval").and_then(Json::arr) {
        Some([a, b]) => match (a.num(), b.num()) {
            (Some(a), Some(b)) => format!(" [{a:.3}-{b:.3}]"),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

/// The speed value of a reading, larger is faster: the median ratio, or for `ts` 1 / its gross
/// median time (for the Windows ratio to Linux x64).
fn speed(id: &str, r: &Json) -> Option<f64> {
    let v = num(r, "value")?;
    if id == TS {
        (v > 0.0).then(|| 1.0 / v)
    } else {
        Some(v)
    }
}

/// A speed cell: the readings `ids` of `builds` from `--gate`.
fn speed_cell(c: &Ctx, ids: &[&str], builds: &[&str]) -> Cell {
    let host = c.o.host.as_str();
    if host == LINUX_A64 {
        return cell(
            Status::NotApplicable,
            "N/A by design: the speed gates hold on macOS arm64 and Linux x64 (09-milestones.md section 2.2); no gate host on Linux a64",
        );
    }
    let Some((p, j)) = &c.gate else {
        return not_given("--gate", GATE_CMD);
    };
    let mut status = Status::Pass;
    let mut why = Vec::new();
    let mut parts = Vec::new();
    let mut gate_hosts = true;
    for &b in builds {
        let Some(blk) = build_block(j, b) else {
            status = status.max(Status::Open);
            why.push(format!(
                "no {b} build run (V12-D6: every M2 speed reading runs in the jit preset and the jitless build)"
            ));
            continue;
        };
        status = status.max(platform_check(
            host,
            blk.path(&["host", "platform"]).and_then(Json::str),
            &format!("the {b} run"),
            &mut why,
        ));
        if let Err(e) = build_line_check(b, blk.get("engine_build").and_then(Json::str)) {
            status = status.max(Status::Open);
            why.push(e);
        }
        if let Err(e) = fresh(
            &c.o.root,
            blk.get("commit").and_then(Json::str),
            blk.get("dirty"),
            &FRESH_PATHS,
        ) {
            status = status.max(Status::Open);
            why.push(format!("the {b} run is stale: {e}"));
        }
        if blk.path(&["host", "gate_host"]).and_then(Json::bool) != Some(true) {
            gate_hosts = false;
        }
        for &id in ids {
            let Some(r) = reading(j, id, b) else {
                status = status.max(Status::Open);
                why.push(format!("no {b} reading {id}"));
                continue;
            };
            let gate = r.get("gate").and_then(Json::str).unwrap_or("no gate field");
            let s = match gate {
                "pass" => Status::Pass,
                "fail" => Status::Fail,
                _ => Status::Open,
            };
            if host != WINDOWS_X64 {
                status = status.max(s);
            } else if r.get("status").and_then(Json::str) != Some("valid") {
                // Windows: no pass or fail against the M2 threshold (the ratio to Linux x64
                // decides, 09:66), so the measurement validity decides, not the gate verdict: a
                // provisional, incomplete or void reading is no reading (M7-P0 Codex r1
                // GATES10-1); a valid reading with no TS rule or a marginal one is read (M7-P0
                // Codex r2 C2-3).
                status = status.max(Status::Open);
            }
            let reasons: Vec<String> = r
                .get("reasons")
                .and_then(Json::arr)
                .unwrap_or(&[])
                .iter()
                .filter_map(Json::str)
                .map(str::to_string)
                .collect();
            let debt = r
                .get("parity_debt")
                .and_then(Json::arr)
                .filter(|d| !d.is_empty())
                .map(|d| {
                    format!(
                        ", parity debt {}",
                        d.iter()
                            .filter_map(Json::str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
                .unwrap_or_default();
            parts.push(format!(
                "{b} {id} {}{}: {gate}{debt}{}",
                num(r, "value").map_or("-".to_string(), |v| format!("{v:.3}")),
                interval_text(r),
                if reasons.is_empty() || gate == "pass" {
                    String::new()
                } else {
                    format!(" ({})", reasons.join("; "))
                }
            ));
        }
    }
    let mut text = format!("{}: {}", p.display(), parts.join("; "));
    if host == LINUX_X64 && !gate_hosts {
        status = status.max(Status::Open);
        text = format!("marginal (no gate host; 10-quality.md section 3): {text}");
    }
    if host == WINDOWS_X64 {
        let (s, t) = windows_ratio(c, ids, builds, j, gate_hosts);
        status = status.max(s);
        text = format!("{t}; the Windows readings: {text}");
    }
    cell(status, format!("{text}{}", because(&why)))
}

/// The Windows x64 cell: each reading against the Linux x64 one (09:66: within 10%).
fn windows_ratio(
    c: &Ctx,
    ids: &[&str],
    builds: &[&str],
    win: &Json,
    win_gate: bool,
) -> (Status, String) {
    let Some((lp, lin)) = &c.linux_gate else {
        return (
            Status::NotRun,
            "no --linux-gate input: the Windows ratio to Linux x64 needs the Linux x64 m2gate JSON"
                .into(),
        );
    };
    let mut ratios = Vec::new();
    let mut status = Status::Pass;
    let mut lin_gate = true;
    let mut same_cpu = true;
    for &b in builds {
        let (wb, lb) = (build_block(win, b), build_block(lin, b));
        let lin_platform = lb
            .and_then(|x| x.path(&["host", "platform"]))
            .and_then(Json::str);
        if lin_platform != Some("linux-x64") {
            status = status.max(Status::Open);
            ratios.push(format!(
                "{b}: {} is not a Linux x64 run ({})",
                lp.display(),
                lin_platform.unwrap_or("no platform")
            ));
        }
        if lb
            .and_then(|x| x.path(&["host", "gate_host"]))
            .and_then(Json::bool)
            != Some(true)
        {
            lin_gate = false;
        }
        let cpu = |x: Option<&Json>| {
            x.and_then(|x| x.path(&["host", "cpu"]))
                .and_then(Json::str)
                .map(str::to_string)
        };
        if cpu(wb).is_none() || cpu(wb) != cpu(lb) {
            same_cpu = false;
        }
        // The Linux x64 run must be valid too, as a --gate run (M7-P0 Codex r1 GATES10-1): its
        // build line, fresh, not provisional, and each reading with a verdict.
        if let Some(lb) = lb {
            if let Err(e) = build_line_check(b, lb.get("engine_build").and_then(Json::str)) {
                status = status.max(Status::Open);
                ratios.push(format!("{b}: the Linux x64 run: {e}"));
            }
            if let Err(e) = fresh(
                &c.o.root,
                lb.get("commit").and_then(Json::str),
                lb.get("dirty"),
                &FRESH_PATHS,
            ) {
                status = status.max(Status::Open);
                ratios.push(format!("{b}: the Linux x64 run is stale: {e}"));
            }
            if lb.get("provisional").and_then(Json::bool) != Some(false) {
                status = status.max(Status::Open);
                ratios.push(format!("{b}: the Linux x64 run is provisional"));
            }
        }
        for &id in ids {
            let w = reading(win, id, b).and_then(|r| speed(id, r));
            let lr = reading(lin, id, b);
            // The Linux x64 reading must be a valid measurement; a marginal one is still read
            // (M7-P0 Codex r2 C2-3).
            let lstatus = lr.and_then(|r| r.get("status")).and_then(Json::str);
            if lstatus != Some("valid") {
                let lgate = lr.and_then(|r| r.get("gate")).and_then(Json::str);
                status = status.max(Status::Open);
                ratios.push(format!(
                    "{b} {id}: the Linux x64 reading is not valid ({}; {})",
                    lstatus.unwrap_or("no status field"),
                    lgate.unwrap_or("no gate field")
                ));
            }
            let l = lr.and_then(|r| speed(id, r));
            match (w, l) {
                (Some(w), Some(l)) if l > 0.0 => {
                    let q = w / l;
                    if q < WINDOWS_MIN {
                        status = status.max(Status::Fail);
                    }
                    ratios.push(format!("{b} {id} Windows / Linux x64 = {q:.3}"));
                }
                _ => {
                    status = status.max(Status::Open);
                    ratios.push(format!("{b} {id}: no value on both hosts"));
                }
            }
        }
    }
    let text = format!(
        "ratio to Linux x64 ({}; >= {WINDOWS_MIN} needed, 09:66): {}",
        lp.display(),
        ratios.join(", ")
    );
    if !(win_gate && lin_gate && same_cpu) {
        // 10:157: Windows x64 ratios use the same machine (dual boot) or identical hardware.
        return (
            status.max(Status::Open),
            format!(
                "marginal (no gate host: a dual-boot or identical gate host is needed, 10-quality.md section 3): {text}"
            ),
        );
    }
    (status, text)
}

/// The extra row: the parity debt of item 2 (09 section 5: a root-cause ADR per kernel).
fn debt_cell(c: &Ctx) -> Cell {
    let Some((_, j)) = &c.gate else {
        return cell(Status::NotRun, "no --gate input");
    };
    let mut debt = BTreeSet::new();
    for b in BUILDS {
        if let Some(d) = reading(j, V8V7, b)
            .and_then(|r| r.get("parity_debt"))
            .and_then(Json::arr)
        {
            for k in d.iter().filter_map(Json::str) {
                debt.insert(format!("{k} ({b})"));
            }
        }
    }
    if debt.is_empty() {
        cell(Status::Pass, "no v8-v7 kernel below 0.8x node --jitless")
    } else {
        cell(
            Status::Open,
            format!(
                "below 0.8x node --jitless: {}; each needs a root-cause ADR (09-milestones.md section 5)",
                debt.into_iter().collect::<Vec<_>>().join(", ")
            ),
        )
    }
}

// ---- item 4: G7 ----------------------------------------------------------------------------

fn item4(c: &Ctx) -> Cell {
    let Some((p, j)) = &c.mem else {
        return not_given("--mem", MEM_CMD);
    };
    let mut status = Status::Pass;
    let mut why = Vec::new();
    status = status.max(platform_check(
        &c.o.host,
        j.path(&["host", "measured", "platform"])
            .and_then(Json::str),
        "the MEMREAD run",
        &mut why,
    ));
    let guard = |when: &str| j.path(&["load", when, "pass"]).and_then(Json::bool);
    if guard("before") != Some(true) || guard("after") != Some(true) {
        status = status.max(Status::Open);
        why.push(
            "the load guard did not pass before and after the run (class Mem needs it, \
             10-quality.md section 3)"
                .into(),
        );
    }
    let rows = j.get("table").and_then(Json::arr).unwrap_or(&[]);
    let row = |kernel: &str, engine: &str| {
        rows.iter().find(|r| {
            r.get("workload").and_then(Json::str) == Some(kernel)
                && r.get("engine").and_then(Json::str) == Some(engine)
        })
    };
    let rss = |r: &Json, what: &str, why: &mut Vec<String>, status: &mut Status| -> Option<f64> {
        let runs = num(r, "n").unwrap_or(0.0);
        if runs < MEM_RUNS {
            *status = (*status).max(Status::Open);
            why.push(format!(
                "{what}: {runs} runs (class Mem needs >= {MEM_RUNS})"
            ));
        }
        num(r, "median_bytes").filter(|x| *x > 0.0)
    };
    let engines = j.get("engines").and_then(Json::arr).unwrap_or(&[]);
    let mut worst: BTreeMap<String, (f64, String)> = BTreeMap::new();
    for b in BUILDS {
        // mem.mjs names the jit preset build `ep9` and the jitless build `ep9-jitless`, and it
        // refuses a binary whose build line names another preset; read the line again here.
        let id = if b == "jit" { "ep9" } else { "ep9-jitless" };
        let line = engines
            .iter()
            .find(|e| e.get("id").and_then(Json::str) == Some(id))
            .and_then(|e| e.get("engine_build"))
            .and_then(Json::str);
        if !line.is_some_and(|l| l.split("; ").any(|w| w == format!("preset {b}"))) {
            status = status.max(Status::Open);
            why.push(format!(
                "engine {id}: its build line {:?} does not name preset {b}",
                line.unwrap_or("none")
            ));
        }
        for k in V8V7_7 {
            let Some(e) = row(k, id) else {
                status = status.max(Status::Open);
                why.push(format!("no ep9 {b} row for {k}"));
                continue;
            };
            let Some(ev) = rss(e, &format!("ep9 {b} {k}"), &mut why, &mut status) else {
                status = status.max(Status::Open);
                why.push(format!("ep9 {b} {k}: no median_bytes"));
                continue;
            };
            for (cmp, cap) in MEM_CAPS {
                let Some(cv) =
                    row(k, cmp).and_then(|r| rss(r, &format!("{cmp} {k}"), &mut why, &mut status))
                else {
                    status = status.max(Status::Open);
                    why.push(format!("no {cmp} row with a peak RSS for {k}"));
                    continue;
                };
                let q = ev / cv;
                if q > cap {
                    status = status.max(Status::Fail);
                    why.push(format!("ep9 {b} {k}: {q:.3}x {cmp} (cap {cap})"));
                }
                let key = format!("{b} vs {cmp} (cap {cap})");
                if worst.get(&key).is_none_or(|(w, _)| q > *w) {
                    worst.insert(key, (q, k.to_string()));
                }
            }
        }
    }
    let worst_text = worst
        .iter()
        .map(|(k, (q, kernel))| format!("{k}: largest {q:.3} ({kernel})"))
        .collect::<Vec<_>>()
        .join("; ");
    with_fresh(
        cell(
            status,
            format!(
                "{}: median peak RSS of ep9 per v8-v7 kernel; {}{}",
                p.display(),
                if worst_text.is_empty() {
                    "no ratio"
                } else {
                    &worst_text
                },
                because(&why)
            ),
        ),
        fresh(
            &c.o.root,
            j.path(&["commit", "ep9"]).and_then(Json::str),
            j.path(&["commit", "runner_dirty"]),
            &FRESH_PATHS,
        ),
    )
}

// ---- item 5: test262 dirs and FASTDIFF -------------------------------------------------------

fn item5(c: &Ctx) -> Cell {
    let t262 = match &c.exec {
        None => not_given("--exec", EXEC_CMD),
        Some((p, j)) => t262_cell(c, p, j),
    };
    let fd = match &c.fastdiff {
        None => not_given("--fastdiff", FASTDIFF_CMD),
        Some((p, j)) => fastdiff_cell(c, p, j),
    };
    cell(
        t262.status.max(fd.status),
        format!(
            "test262 {}: {}; FASTDIFF {}: {}",
            t262.status.name(),
            t262.evidence,
            fd.status.name(),
            fd.evidence
        ),
    )
}

fn t262_cell(c: &Ctx, p: &Path, j: &Json) -> Cell {
    let mut status = Status::Pass;
    let mut why = Vec::new();
    if j.get("gc_stress").and_then(Json::num).unwrap_or(0.0) != 0.0 {
        status = Status::Fail;
        why.push("a GC-stress run; give the default run".into());
    }
    let (infra, _, _) = exec_health(j);
    if infra > 0 {
        status = Status::Fail;
        why.push(format!("{infra} infrastructure errors"));
    }
    // G1: a file passes when its result is a pass. The JSON lists the files that do not pass
    // (`results`), and a skipped file counts as a failure.
    let mut not_pass: BTreeSet<&str> = j
        .get("results")
        .and_then(Json::arr)
        .unwrap_or(&[])
        .iter()
        .filter(|r| r.get("status").and_then(Json::str) != Some("pass"))
        .filter_map(|r| r.get("path").and_then(Json::str))
        .collect();
    not_pass.extend(
        j.get("skipped_paths")
            .and_then(Json::arr)
            .unwrap_or(&[])
            .iter()
            .filter_map(Json::str),
    );
    let files = match test262_files(&c.o.root.join("third_party/test262")) {
        Ok(f) => Some(f),
        Err(e) => {
            status = status.max(Status::Open);
            why.push(format!("the dir file counts cannot be read: {e}"));
            None
        }
    };
    // Coverage (M7-P0 Codex r1 GATES10-3): the run ran every file of each gated dir of this
    // checkout. A filtered run names the files it ran (`run_paths`): each file of a gated dir
    // is there or in `skipped_paths`, else OPEN. A full run has no list: its `by_dir` count of
    // each top dir (language, annexB, built-ins) is the checkout's (missing: OPEN; another
    // count: FAIL).
    let run_paths: Option<BTreeSet<&str>> = j
        .get("run_paths")
        .and_then(Json::arr)
        .map(|a| a.iter().filter_map(Json::str).collect());
    let skipped: BTreeSet<&str> = j
        .get("skipped_paths")
        .and_then(Json::arr)
        .unwrap_or(&[])
        .iter()
        .filter_map(Json::str)
        .collect();
    if let (None, Some(files)) = (&run_paths, &files) {
        for key in ["language", "annexB", "built-ins"] {
            let prefix = format!("test/{key}/");
            let total = files.iter().filter(|f| f.starts_with(&prefix)).count() as u64;
            match count(j, &["by_dir", key, "files"]) {
                Some((_, t)) if t == total => {}
                Some((_, t)) => {
                    status = Status::Fail;
                    why.push(format!(
                        "{t} {key} files in the run, the checkout has {total} (a filtered run?)"
                    ));
                }
                None => {
                    status = status.max(Status::Open);
                    why.push(format!(
                        "the run has no by_dir {key} count and no run_paths list (a filtered run?)"
                    ));
                }
            }
        }
    }
    let mut dirs = Vec::new();
    for (dir, need) in T262_DIRS {
        let prefix = format!("test/{dir}/");
        let Some(files) = &files else {
            continue;
        };
        let total = files.iter().filter(|f| f.starts_with(&prefix)).count() as u64;
        let failed = not_pass.iter().filter(|f| f.starts_with(&prefix)).count() as u64;
        let passed = total.saturating_sub(failed);
        if let Some(run) = &run_paths {
            let missed = files
                .iter()
                .filter(|f| f.starts_with(&prefix))
                .filter(|f| !run.contains(f.as_str()) && !skipped.contains(f.as_str()))
                .count();
            if missed > 0 {
                status = status.max(Status::Open);
                why.push(format!("{dir}: {missed} files not run (a filtered run)"));
            }
        }
        let ok = total > 0 && passed * 100 >= total * need;
        if !ok {
            status = status.max(Status::Fail);
        }
        dirs.push(format!(
            "{dir}/ {passed} / {total} = {:.2}% (>= {need}%){}",
            pct(passed, total),
            if ok { "" } else { " FAIL" }
        ));
    }
    let src = match fresh(
        &c.o.root,
        j.get("commit").and_then(Json::str),
        j.get("dirty"),
        &T262_PATHS,
    ) {
        Ok(t) => Ok(t),
        Err(_)
            if j.get("dirty").and_then(Json::bool) == Some(false)
                && same_trees(&c.o.root, j.get("tree"), &T262_PATHS) =>
        {
            Ok("its tree ids of the measured paths equal HEAD's".into())
        }
        Err(e) => Err(e),
    };
    with_fresh(
        cell(
            status,
            format!(
                "{} (preset {}): {}, infra {infra}{}",
                p.display(),
                j.get("preset").and_then(Json::str).unwrap_or("?"),
                dirs.join(", "),
                because(&why)
            ),
        ),
        src,
    )
}

fn fastdiff_cell(c: &Ctx, p: &Path, j: &Json) -> Cell {
    let paths = j.get("fast_paths").and_then(Json::arr).unwrap_or(&[]);
    let mut status = if paths.is_empty() {
        Status::Open
    } else {
        Status::Pass
    };
    let mut why = Vec::new();
    if paths.is_empty() {
        why.push("no fast path listed".into());
    }
    let (mut cases, mut mismatches) = (0.0, 0.0);
    let mut seen = BTreeSet::new();
    for f in paths {
        let name = f.get("name").and_then(Json::str).unwrap_or("?");
        if !seen.insert(name) {
            status = status.max(Status::Open);
            why.push(format!("fast path {name:?} listed twice"));
        }
        let n = num(f, "cases").unwrap_or(0.0);
        let m = num(f, "mismatches");
        cases += n;
        if n < 1.0 {
            status = status.max(Status::Open);
            why.push(format!("{name}: 0 cases"));
        }
        match m {
            Some(0.0) => {}
            Some(m) => {
                mismatches += m;
                status = Status::Fail;
                why.push(format!("{name}: {m} mismatches"));
            }
            None => {
                status = status.max(Status::Open);
                why.push(format!("{name}: no mismatch count"));
            }
        }
    }
    with_fresh(
        cell(
            status,
            format!(
                "{}: {} fast paths, {cases} cases, {mismatches} mismatches{}",
                p.display(),
                paths.len(),
                because(&why)
            ),
        ),
        fresh(
            &c.o.root,
            j.get("commit").and_then(Json::str),
            j.get("dirty"),
            &ENGINE_PATHS,
        ),
    )
}

// ---- item 7: JetStream ----------------------------------------------------------------------

/// The workload names of a list file (`#` comments and blank lines dropped).
fn list_names(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn item7(c: &Ctx) -> Cell {
    let Some((p, j)) = &c.jetstream else {
        return not_given("--jetstream", JETSTREAM_CMD);
    };
    let mut status = Status::Pass;
    let mut why = Vec::new();
    let names = match read(&c.o.root.join(JETSTREAM_LIST)) {
        Ok(t) => list_names(&t),
        Err(e) => return cell(Status::Open, format!("{}: {e}", p.display())),
    };
    if j.get("list")
        .and_then(Json::str)
        .map(|l| l.replace('\\', "/"))
        .as_deref()
        != Some(JETSTREAM_LIST)
    {
        status = status.max(Status::Open);
        why.push(format!(
            "the run read the list {:?}, not {JETSTREAM_LIST}",
            j.get("list").and_then(Json::str).unwrap_or("none")
        ));
    }
    let pin = read(&c.o.root.join(JETSTREAM_PIN))
        .map(|t| t.trim().to_string())
        .unwrap_or_default();
    if j.get("jetstream_commit").and_then(Json::str) != Some(pin.as_str()) {
        status = status.max(Status::Open);
        why.push(format!(
            "the run's JetStream commit is not the pin {}",
            short(&pin)
        ));
    }
    let node = format!(
        "{}-{}",
        j.path(&["host", "platform"])
            .and_then(Json::str)
            .unwrap_or("?"),
        j.path(&["host", "arch"]).and_then(Json::str).unwrap_or("?")
    );
    status = status.max(platform_check(
        &c.o.host,
        j.get("host").map(|_| node.as_str()),
        "the JetStream run",
        &mut why,
    ));
    let reps = num(j, "reps").unwrap_or(0.0);
    let configs = j.get("configs").and_then(Json::arr).unwrap_or(&[]);
    let cname = |x: &Json| x.get("name").and_then(Json::str).unwrap_or("").to_string();
    // run.mjs keys the table and the summary by config name, so two configs of one name share
    // their scores (M7-P0 Codex r3 C3-1): such a run is no reading.
    let mut seen = BTreeSet::new();
    for n in configs.iter().map(cname) {
        if !seen.insert(n.clone()) {
            status = status.max(Status::Open);
            why.push(format!(
                "two configs have the name {n:?} (run.mjs merges their scores)"
            ));
        }
    }
    // An ep9 config is one that run.mjs ran on an ep9 binary (kind `ep9`, through ep9-host.mjs),
    // not one by its name: a d8 or node engine under the name `ep9` read PASS (M7-P0 Codex r2
    // C2-2 part A). Its version must hold the build line of `ep9 --version`.
    let subjects: Vec<String> = configs
        .iter()
        .filter(|x| x.get("kind").and_then(Json::str) == Some("ep9"))
        .map(cname)
        .collect();
    for x in configs
        .iter()
        .filter(|x| x.get("kind").and_then(Json::str) == Some("ep9"))
    {
        let v = x.get("version").and_then(Json::str);
        if !v.is_some_and(|v| v.contains("; features ") && v.contains("; preset ")) {
            status = status.max(Status::Open);
            why.push(format!(
                "{}: the version has no ep9 build line (`features <list>; preset <name>`): {}",
                cname(x),
                v.unwrap_or("null")
            ));
        }
    }
    let jitless = configs
        .iter()
        .find(|x| {
            // `node` (this node) or `node@<path>`: run.mjs writes the kind of both as `node`.
            (x.get("engine").and_then(Json::str) == Some("node")
                || x.get("kind").and_then(Json::str) == Some("node"))
                && x.get("flags")
                    .and_then(Json::arr)
                    .is_some_and(|f| f.iter().any(|f| f.str() == Some("--jitless")))
        })
        .map(cname);
    if subjects.is_empty() {
        status = status.max(Status::Open);
        why.push("no ep9 config (a config of kind \"ep9\": engine ep9 or ep9@<path>)".into());
    }
    let Some(jitless) = jitless else {
        return cell(
            status.max(Status::Open),
            format!("{}: no node --jitless config{}", p.display(), because(&why)),
        );
    };
    let table = j.get("table").and_then(Json::arr).unwrap_or(&[]);
    let mut parts = Vec::new();
    for s in &subjects {
        let mut ran = 0usize;
        let mut short_of = Vec::new();
        for n in &names {
            let row = table
                .iter()
                .find(|r| r.get("name").and_then(Json::str) == Some(n.as_str()));
            let got = |cfg: &str| {
                row.and_then(|r| {
                    let full = num(r, &format!("{cfg}_n")) == Some(reps) && reps >= 1.0;
                    num(r, cfg).filter(|x| full && *x > 0.0)
                })
            };
            match (got(s), got(&jitless)) {
                (Some(_), Some(_)) => ran += 1,
                _ => short_of.push(n.clone()),
            }
        }
        if !short_of.is_empty() {
            status = status.max(Status::Open);
            why.push(format!(
                "{s}: {} of {} workloads lack {reps} scores on {s} or {jitless}: {}",
                short_of.len(),
                names.len(),
                some_paths(&short_of)
            ));
        }
        // The runner's list geomean (summary.<config>.geomean_score, null unless complete): over
        // one workload set, geomean(a) / geomean(b) is the geomean of the ratios (xtask computes
        // no logarithm: H36 bans platform libm in this workspace).
        let gm = |cfg: &str| {
            j.path(&["summary", cfg, "geomean_score"])
                .and_then(Json::num)
                .filter(|x| *x > 0.0)
        };
        let ratio = match (gm(s), gm(&jitless)) {
            (Some(a), Some(b)) if short_of.is_empty() => Some(a / b),
            _ => None,
        };
        parts.push(format!(
            "{s}: {} of {} workloads ran, geomean score ratio to {jitless} {}",
            ran,
            names.len(),
            ratio.map_or("-".to_string(), |r| format!("{r:.3}"))
        ));
    }
    with_fresh(
        cell(
            status,
            format!("{}: {}{}", p.display(), parts.join("; "), because(&why)),
        ),
        fresh(
            &c.o.root,
            j.path(&["ep9", "commit"]).and_then(Json::str),
            j.path(&["ep9", "dirty"]),
            &FRESH_PATHS,
        ),
    )
}

// ---- item 8: Fuzzilli -------------------------------------------------------------------------

fn item8(c: &Ctx) -> Cell {
    if c.fuzz.is_empty() {
        return not_given("--fuzz", FUZZ_HOW);
    }
    let mut status = Status::Pass;
    let mut why = Vec::new();
    let mut hours = 0.0;
    let mut open: Vec<String> = Vec::new();
    let mut runs = Vec::new();
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
        // Every record's open crashes count (G6); only Fuzzilli hours count for item 8.
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
        let h = num(j, "cpu_hours").unwrap_or(0.0);
        let commit = j.get("commit").and_then(Json::str);
        let campaign = j
            .get("campaign")
            .and_then(Json::str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let mut counted = tool == "fuzzilli" && j.get("counted").and_then(Json::bool) == Some(true);
        if counted {
            let key = campaign.map(|id| format!("{tool}/{id}"));
            let reason = match &key {
                None => Some("a counted record names no campaign id".to_string()),
                Some(k) if campaigns.contains(k) => {
                    Some(format!("campaign {k} again, its hours counted once"))
                }
                Some(_) => fresh(&c.o.root, commit, j.get("dirty"), &FUZZ_PATHS).err(),
            };
            if let Some(r) = reason {
                counted = false;
                status = status.max(Status::Open);
                why.push(format!("{}: not counted: {r}", p.display()));
            } else {
                campaigns.extend(key);
                hours += h;
            }
        }
        runs.push(format!(
            "{} {tool}{} {h:.1} CPU-h at `{}`{}",
            p.display(),
            campaign.map_or(String::new(), |id| format!(" campaign {id:?}")),
            short(commit.unwrap_or("unknown")),
            if counted { "" } else { " (not counted)" }
        ));
    }
    if hours < FUZZILLI_HOURS {
        status = status.max(Status::Open);
        why.push(format!(
            "fuzzilli: {hours:.1} counted CPU-h of {FUZZILLI_HOURS}"
        ));
    }
    if !open.is_empty() {
        status = Status::Fail;
        why.push(format!("open unique crashes: {}", some_paths(&open)));
    }
    cell(
        status,
        format!(
            "{}; Fuzzilli {hours:.1} / {FUZZILLI_HOURS} CPU-h, {} open unique crashes{}",
            runs.join(", "),
            open.len(),
            because(&why)
        ),
    )
}

// ---- item 9: P-gates ---------------------------------------------------------------------------

/// The rows of the first Markdown table with the columns `Gate` and `Result`: (gate, result).
fn p_gate_rows(md: &str) -> Vec<(String, String)> {
    let cells = |l: &str| -> Vec<String> {
        l.trim()
            .trim_matches('|')
            .split('|')
            .map(|c| c.trim().trim_matches('`').trim().to_string())
            .collect()
    };
    let mut out = Vec::new();
    let mut cols: Option<(usize, usize)> = None;
    for line in md.lines() {
        let l = line.trim();
        if !l.starts_with('|') {
            if cols.is_some() && !out.is_empty() {
                break;
            }
            continue;
        }
        let cs = cells(l);
        if cs
            .iter()
            .all(|c| c.chars().all(|ch| matches!(ch, '-' | ':')))
        {
            continue;
        }
        match cols {
            None => {
                let g = cs.iter().position(|c| c.eq_ignore_ascii_case("gate"));
                let r = cs.iter().position(|c| c.eq_ignore_ascii_case("result"));
                if let (Some(g), Some(r)) = (g, r) {
                    cols = Some((g, r));
                }
            }
            Some((g, r)) => {
                if let (Some(gv), Some(rv)) = (cs.get(g), cs.get(r)) {
                    out.push((gv.clone(), rv.to_lowercase()));
                }
            }
        }
    }
    out
}

fn item9(c: &Ctx) -> Cell {
    let Some((p, md)) = &c.p_gates else {
        return not_given("--p-gates", P_GATES_HOW);
    };
    let rows = p_gate_rows(md);
    let mut status = Status::Pass;
    let mut why = Vec::new();
    let mut parts = Vec::new();
    for g in P_GATES {
        let mine: Vec<&(String, String)> = rows.iter().filter(|(x, _)| x == g).collect();
        match mine.as_slice() {
            [(_, r)] if P_RESULTS.contains(&r.as_str()) => parts.push(format!("{g} {r}")),
            [(_, r)] => {
                status = status.max(Status::Open);
                why.push(format!(
                    "{g}: result {r:?} is not `not tripped` or `tripped`"
                ));
            }
            [] => {
                status = status.max(Status::Open);
                why.push(format!("no {g} row"));
            }
            _ => {
                status = status.max(Status::Open);
                why.push(format!("{g}: {} rows", mine.len()));
            }
        }
    }
    let committed = git(&c.o.root, &["ls-files", "--", &p.to_string_lossy()]).is_some();
    if !committed {
        why.push("the file is not committed in this tree".into());
        status = status.max(Status::Open);
    }
    cell(
        status,
        format!(
            "{}: {} (01-front-end.md P-gates; a tripped gate starts option D){}",
            p.display(),
            if parts.is_empty() {
                "no gate read".to_string()
            } else {
                parts.join(", ")
            },
            because(&why)
        ),
    )
}

/// The extra row: P-gates that tripped.
fn trip_cell(c: &Ctx) -> Cell {
    let Some((_, md)) = &c.p_gates else {
        return cell(Status::NotRun, "no --p-gates input");
    };
    let tripped: Vec<String> = p_gate_rows(md)
        .into_iter()
        .filter(|(g, r)| P_GATES.contains(&g.as_str()) && r == "tripped")
        .map(|(g, _)| g)
        .collect();
    if tripped.is_empty() {
        cell(Status::Pass, "no P-gate tripped")
    } else {
        cell(
            Status::Open,
            format!(
                "tripped: {}; option D needs a decisions-log row (01-front-end.md P-gates)",
                tripped.join(", ")
            ),
        )
    }
}

// ---- render ----------------------------------------------------------------------------------

fn render(
    c: &Ctx,
    cells: Vec<Cell>,
    extra: Vec<(&'static str, Cell)>,
    missing: Vec<String>,
) -> Report {
    let o = c.o;
    let head = git(&o.root, &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let mut table = String::new();
    let _ = writeln!(table, "| # | M2 exit item | {} |", o.host);
    let _ = writeln!(table, "|---|---|---|");
    for ((l, t), h) in LABELS.iter().zip(TITLES).zip(&cells) {
        let _ = writeln!(table, "| {l} | {t} | {} |", h.status.name());
    }
    let all_pass = cells.iter().all(|c| c.status.ok());
    let mut md = String::from("# M2 exit check (`cargo xtask m2-exit`)\n\n");
    let _ = writeln!(
        md,
        "- Items: docs/plan/09-milestones.md section 2.2, M2 exit 1-10, and the D11 row (11-decisions.md D11; plan v12 V12-D6)."
    );
    let _ = writeln!(md, "- Host column: {}; tree at commit `{head}`.", o.host);
    let _ = writeln!(
        md,
        "- Result: {}.\n",
        if all_pass {
            "every item PASS (N/A cells are by design): the M2 exit holds on this host"
        } else {
            "not every cell is PASS"
        }
    );
    md.push_str(&table);
    md.push_str("\n## Evidence\n");
    for ((l, t), h) in LABELS.iter().zip(TITLES).zip(&cells) {
        let _ = write!(
            md,
            "\n### {l}. {t}\n\n- {} {}: {}\n",
            o.host,
            h.status.name(),
            h.evidence
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
    /// The commit placeholder of the fixtures (`tests/m2_exit/`).
    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn fixture(name: &str) -> String {
        read(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/m2_exit")
                .join(name),
        )
        .unwrap()
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
                "user.name=m2-exit test",
                "-c",
                "user.email=m2-exit@test.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    /// A scratch repository with the paths the items read, one commit, and an untracked test262
    /// checkout: language 20 files, annexB 10, each of the 8 built-ins dirs 20.
    struct Repo {
        root: PathBuf,
        head: String,
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn repo(name: &str) -> Repo {
        let root = std::env::temp_dir().join(format!("ep9-m2-exit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        git_in(&root, &["init", "-q"]);
        for p in [
            "crates/ep9_vm/src/lib.rs",
            "tools/bench/README.md",
            "tools/test262/src/main.rs",
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "third_party/regress.commit",
            "third_party/patches/regress/0001.patch",
            "third_party/test262.commit",
        ] {
            write(&root, p, "x\n");
        }
        write(
            &root,
            JETSTREAM_PIN,
            "06785cf861ac44855f168cbbe829278c2802e6de\n",
        );
        write(&root, JETSTREAM_LIST, "# list\nai-astar\nAir\nBabylon\n");
        write(
            &root,
            "docs/measurements/2026-11-06-p-gates.md",
            &fixture("p-gates.md"),
        );
        // test262 is git-ignored in the real tree.
        write(&root, ".gitignore", "/third_party/test262/\n");
        git_in(&root, &["add", "-A"]);
        git_in(&root, &["commit", "-q", "-m", "base"]);
        let t = |rel: String| write(&root, &format!("third_party/test262/test/{rel}"), "//\n");
        (0..20).for_each(|i| t(format!("language/l{i}.js")));
        (0..10).for_each(|i| t(format!("annexB/a{i}.js")));
        for d in [
            "Array", "String", "Object", "Function", "Math", "JSON", "Number", "RegExp",
        ] {
            let c = d[..1].to_lowercase();
            (0..20).for_each(|i| t(format!("built-ins/{d}/{c}{i}.js")));
        }
        let head = git_in(&root, &["rev-parse", "HEAD"]);
        Repo { root, head }
    }

    /// A fixture with the commit placeholder set to `head`, written into the scratch dir.
    fn input(r: &Repo, name: &str) -> PathBuf {
        let p = r.root.join("inputs").join(name);
        write(
            &r.root,
            &format!("inputs/{name}"),
            &fixture(name).replace(SHA, &r.head),
        );
        p
    }

    fn opts(r: &Repo, host: &str) -> Options {
        Options {
            root: r.root.clone(),
            host: host.into(),
            out: r.root.join("m2-exit.md"),
            ..Options::default()
        }
    }

    fn all(r: &Repo, gate: &str) -> Options {
        Options {
            gate: Some(input(r, gate)),
            mem: Some(input(r, "mem.json")),
            exec: Some(input(r, "exec.json")),
            fastdiff: Some(input(r, "fastdiff.json")),
            jetstream: Some(input(r, "jetstream.json")),
            fuzz: vec![input(r, "fuzz-fuzzilli.json")],
            p_gates: Some(r.root.join("docs/measurements/2026-11-06-p-gates.md")),
            ..opts(r, HOST)
        }
    }

    fn statuses(rep: &Report) -> Vec<&'static str> {
        rep.cells.iter().map(|c| c.status.name()).collect()
    }

    #[test]
    fn every_cell_line_and_the_missing_inputs_with_no_input() {
        let r = repo("none");
        let rep = run(&opts(&r, HOST)).unwrap();
        assert_eq!(rep.table.lines().count(), 2 + 11, "{}", rep.table);
        for (l, t) in LABELS.iter().zip(TITLES) {
            assert!(
                rep.table.contains(&format!("| {l} | {t} | NOT-RUN |")),
                "{l}: {}",
                rep.table
            );
        }
        assert_eq!(rep.missing.len(), 7, "{:?}", rep.missing);
        assert!(
            rep.markdown
                .contains(&format!("tree at commit `{}`", r.head))
        );
        // Linux a64: the speed cells are N/A by design and need no --gate.
        let rep = run(&opts(&r, LINUX_A64)).unwrap();
        assert_eq!(
            statuses(&rep),
            [
                "N/A", "N/A", "N/A", "NOT-RUN", "NOT-RUN", "N/A", "NOT-RUN", "NOT-RUN", "NOT-RUN",
                "N/A", "N/A"
            ]
        );
        assert_eq!(rep.missing.len(), 6);
    }

    #[test]
    fn all_inputs_on_the_mac() {
        let r = repo("all");
        let rep = run(&all(&r, "gate-pass.json")).unwrap();
        // Item 8: 260.5 of 500 Fuzzilli CPU-h. Everything else passes.
        assert_eq!(
            statuses(&rep),
            [
                "PASS", "PASS", "PASS", "PASS", "PASS", "PASS", "PASS", "OPEN", "PASS", "PASS",
                "PASS"
            ],
            "{}",
            rep.markdown
        );
        let ev = |i: usize| rep.cells[i].evidence.clone();
        assert!(ev(2).contains("jit ts 2.500"), "{}", ev(2));
        assert!(ev(4).contains("language/ 19 / 20 = 95.00%"), "{}", ev(4));
        assert!(ev(4).contains("annexB/ 9 / 10 = 90.00%"), "{}", ev(4));
        assert!(
            ev(4).contains("built-ins/Math/ 19 / 20"),
            "a skipped file fails (G1): {}",
            ev(4)
        );
        assert!(
            ev(4).contains("3 fast paths, 22 cases, 0 mismatches"),
            "{}",
            ev(4)
        );
        assert!(
            ev(6).contains("ep9: 3 of 3 workloads ran, geomean score ratio to jitless 0.800"),
            "{}",
            ev(6)
        );
        assert!(ev(7).contains("260.5 counted CPU-h of 500"), "{}", ev(7));
        assert!(
            ev(8).contains("P1 not tripped, P2 not tripped, P4 tripped"),
            "{}",
            ev(8)
        );
        assert!(
            ev(3).contains("jit vs node-jitless (cap 1.2): largest 1.100 (splay)"),
            "{}",
            ev(3)
        );
        let extra: Vec<_> = rep
            .extra
            .iter()
            .map(|(w, c)| (*w, c.status.name()))
            .collect();
        assert_eq!(
            extra,
            [("parity debt", "PASS"), ("P-gates tripped", "OPEN")]
        );
    }

    #[test]
    fn a_failed_build_fails_the_cell_and_both_builds_are_needed() {
        let r = repo("mixed");
        let rep = run(&all(&r, "gate-mixed.json")).unwrap();
        let s = statuses(&rep);
        // The jitless Octane-7 fails items 1 and D11; its TS is marginal (item 3 OPEN).
        assert_eq!(
            (s[0], s[1], s[2], s[5], s[9], s[10]),
            ("FAIL", "PASS", "OPEN", "PASS", "PASS", "FAIL")
        );
        assert!(
            rep.cells[1].evidence.contains("parity debt raytrace"),
            "{}",
            rep.cells[1].evidence
        );
        assert_eq!(rep.extra[0].1.status, Status::Open);
        // One build only: OPEN (V12-D6).
        let mut j = fixture("gate-pass.json").replace(SHA, &r.head);
        j = j.replacen("\"build\": \"jitless\"", "\"build\": \"other\"", 1);
        write(&r.root, "inputs/one.json", &j);
        let o = Options {
            gate: Some(r.root.join("inputs/one.json")),
            ..opts(&r, HOST)
        };
        let rep = run(&o).unwrap();
        assert_eq!(rep.cells[0].status, Status::Open);
        assert!(
            rep.cells[0].evidence.contains("no jitless build run"),
            "{}",
            rep.cells[0].evidence
        );
    }

    #[test]
    fn stale_dirty_or_wrong_host_inputs() {
        let r = repo("stale");
        let mut o = all(&r, "gate-pass.json");
        // A later commit that changes crates/: every speed and MEMREAD input is stale.
        write(&r.root, "crates/ep9_vm/src/lib.rs", "y\n");
        git_in(&r.root, &["commit", "-q", "-am", "change"]);
        let rep = run(&o).unwrap();
        for i in [0, 1, 2, 3, 4, 5, 6, 9, 10] {
            assert_eq!(
                rep.cells[i].status,
                Status::Open,
                "{i}: {}",
                rep.cells[i].evidence
            );
            assert!(
                rep.cells[i].evidence.contains("changed between"),
                "{i}: {}",
                rep.cells[i].evidence
            );
        }
        // Another host: the platform check fails (X12).
        o.host = LINUX_X64.into();
        o.exec = None;
        let rep = run(&o).unwrap();
        assert_eq!(rep.cells[0].status, Status::Fail);
        assert!(
            rep.cells[0]
                .evidence
                .contains("measured on darwin-arm64, not linux-x64")
        );
        // Gate-host runs: no "marginal (no gate host)" prefix.
        assert!(
            !rep.cells[0].evidence.starts_with("marginal (no gate host"),
            "{}",
            rep.cells[0].evidence
        );
    }

    #[test]
    fn linux_x64_without_a_gate_host_reads_marginal() {
        let r = repo("linux");
        let o = Options {
            gate: Some(input(&r, "gate-linux.json")),
            ..opts(&r, LINUX_X64)
        };
        let rep = run(&o).unwrap();
        for i in [0, 1, 2, 5, 9, 10] {
            assert_eq!(rep.cells[i].status, Status::Open);
            assert!(
                rep.cells[i].evidence.starts_with("marginal (no gate host"),
                "{}",
                rep.cells[i].evidence
            );
        }
    }

    #[test]
    fn windows_gives_the_ratio_to_linux_x64_and_reads_marginal() {
        let r = repo("windows");
        let lin = input(&r, "gate-linux.json");
        let win = fixture("gate-linux.json")
            .replace(SHA, &r.head)
            .replace("\"platform\": \"linux-x64\"", "\"platform\": \"win32-x64\"");
        write(&r.root, "inputs/gate-win.json", &win);
        let o = Options {
            gate: Some(r.root.join("inputs/gate-win.json")),
            linux_gate: Some(lin),
            ..opts(&r, WINDOWS_X64)
        };
        let rep = run(&o).unwrap();
        let c = &rep.cells[0];
        assert_eq!(c.status, Status::Open, "{}", c.evidence);
        assert!(
            c.evidence
                .starts_with("marginal (no gate host: a dual-boot or identical gate host"),
            "{}",
            c.evidence
        );
        assert!(
            c.evidence
                .contains("jit octane7-vs-qjs Windows / Linux x64 = 1.000"),
            "{}",
            c.evidence
        );
        // Without --linux-gate: NOT-RUN, and a missing-input line.
        let o = Options {
            gate: Some(r.root.join("inputs/gate-win.json")),
            ..opts(&r, WINDOWS_X64)
        };
        let rep = run(&o).unwrap();
        assert_eq!(rep.cells[0].status, Status::NotRun);
        assert!(rep.missing.iter().any(|m| m.starts_with("--linux-gate")));
        assert!(
            parse_args(
                &[
                    "--host".into(),
                    HOST.into(),
                    "--out".into(),
                    "x".into(),
                    "--linux-gate".into(),
                    "y".into()
                ],
                PathBuf::new()
            )
            .is_err()
        );
    }

    /// M7-P0 Codex r1 GATES10-1: gate hosts and one CPU on both sides, but Windows readings
    /// with no verdict (a provisional run) and a Linux x64 run that is dirty, names another
    /// commit and has a wrong jitless build line: every Windows speed cell reads OPEN (it read
    /// PASS, ratio 1.000).
    #[test]
    fn windows_needs_valid_readings_on_both_hosts() {
        let r = repo("windows-valid");
        let gate_host = |t: String| t.replace("\"gate_host\": false", "\"gate_host\": true");
        let lin = gate_host(fixture("gate-linux.json"))
            .replace("\"dirty\": false", "\"dirty\": true")
            .replace(
                "\"engine_build\": \"features none; preset jitless\"",
                "\"engine_build\": \"features jit; preset jit\"",
            );
        write(
            &r.root,
            "inputs/gate-lin.json",
            &lin.replace(SHA, &"f".repeat(40)),
        );
        let win = gate_host(fixture("gate-linux.json"))
            .replace(SHA, &r.head)
            .replace("\"platform\": \"linux-x64\"", "\"platform\": \"win32-x64\"")
            .replace("\"provisional\": false", "\"provisional\": true");
        write(&r.root, "inputs/gate-win.json", &win);
        let o = Options {
            gate: Some(r.root.join("inputs/gate-win.json")),
            linux_gate: Some(r.root.join("inputs/gate-lin.json")),
            ..opts(&r, WINDOWS_X64)
        };
        let rep = run(&o).unwrap();
        for i in [0, 1, 2, 5, 9, 10] {
            let c = &rep.cells[i];
            assert_eq!(c.status, Status::Open, "cell {i}: {}", c.evidence);
            for want in [
                "the Linux x64 reading is not valid",
                "the Linux x64 run is stale",
            ] {
                assert!(
                    c.evidence.contains(want),
                    "cell {i}: {want}: {}",
                    c.evidence
                );
            }
        }
        let c = &rep.cells[0];
        assert!(
            c.evidence
                .contains("jitless: the Linux x64 run: the jitless run's build line"),
            "{}",
            c.evidence
        );
        // Valid Linux x64 readings (pass, fresh, not provisional), the Windows readings still
        // with no verdict: OPEN from the Windows side alone.
        let lin = gate_host(fixture("gate-pass.json"))
            .replace(SHA, &r.head)
            .replace(
                "\"platform\": \"darwin-arm64\"",
                "\"platform\": \"linux-x64\"",
            )
            .replace("\"cpu\": \"Apple M5 Max\"", "\"cpu\": \"AMD EPYC 7763\"");
        write(&r.root, "inputs/gate-lin.json", &lin);
        let rep = run(&o).unwrap();
        for i in [0, 1, 2, 5, 9, 10] {
            let c = &rep.cells[i];
            assert_eq!(c.status, Status::Open, "cell {i}: {}", c.evidence);
            assert!(
                !c.evidence.contains("the Linux x64"),
                "cell {i}: {}",
                c.evidence
            );
        }
    }

    /// M7-P0 Codex r2 C2-3: a Windows TS reading has no TS rule (no gate verdict, V12-D5), and a
    /// valid "marginal" geomean reading on Windows or on the Linux x64 side has no pass verdict.
    /// Both read OPEN; the measurement validity decides now, so the ratio cells read PASS.
    #[test]
    fn windows_ts_reads_the_ratio_without_a_ts_rule() {
        let r = repo("windows-ts");
        let o = Options {
            gate: Some(input(&r, "gate-win.json")),
            linux_gate: Some(input(&r, "gate-win-linux.json")),
            ..opts(&r, WINDOWS_X64)
        };
        let rep = run(&o).unwrap();
        for i in [0, 1, 2, 5, 9, 10] {
            let c = &rep.cells[i];
            assert_eq!(c.status, Status::Pass, "cell {i}: {}", c.evidence);
            assert!(
                !c.evidence.contains("not valid"),
                "cell {i}: {}",
                c.evidence
            );
        }
        let ts = &rep.cells[2].evidence;
        for want in [
            "jit ts Windows / Linux x64 = 1.000",
            "jitless ts Windows / Linux x64 = 1.000",
            "no gate verdict (no TS rule for host win-gate (win32-x64)",
        ] {
            assert!(ts.contains(want), "{want}: {ts}");
        }
        let oct = &rep.cells[0].evidence;
        assert!(
            oct.contains("jitless octane7-vs-qjs Windows / Linux x64 = 0.950"),
            "{oct}"
        );
        assert!(
            oct.contains("jit octane7-vs-qjs Windows / Linux x64 = 1.053"),
            "{oct}"
        );
    }

    /// M7-P0 Codex r1 GATES10-3: a run of language/ and annexB/ only (no built-ins result, no
    /// skipped built-ins file, no by_dir built-ins count) read PASS with every built-ins dir at
    /// 20 / 20. Now OPEN; and a filtered run (run_paths) that misses one Math file is OPEN.
    #[test]
    fn item5_a_run_without_built_ins_is_not_a_pass() {
        let r = repo("t262-cover");
        let full = fixture("exec.json").replace(SHA, &r.head);
        let no_bi = full.replace(
            "  },\n  \"built-ins\": {\n   \"files\": {\n    \"passed\": 157,\n    \"total\": 160\n   }\n  }\n",
            "  }\n",
        );
        assert_ne!(no_bi, full, "the fixture has a by_dir built-ins count");
        write(&r.root, "inputs/exec-nobi.json", &no_bi);
        let o = Options {
            exec: Some(r.root.join("inputs/exec-nobi.json")),
            ..all(&r, "gate-pass.json")
        };
        let c = &run(&o).unwrap().cells[4];
        assert_eq!(c.status, Status::Open, "{}", c.evidence);
        assert!(
            c.evidence.contains("no by_dir built-ins count"),
            "{}",
            c.evidence
        );
        // A filtered run that names every file but one Math file.
        let mut ran: Vec<String> = test262_files(&r.root.join("third_party/test262"))
            .unwrap()
            .into_iter()
            .filter(|f| f != "test/built-ins/Math/m3.js")
            .map(|f| format!("\"{f}\""))
            .collect();
        ran.sort();
        let filtered = full.replacen('{', &format!("{{\"run_paths\": [{}], ", ran.join(", ")), 1);
        write(&r.root, "inputs/exec-run.json", &filtered);
        let o = Options {
            exec: Some(r.root.join("inputs/exec-run.json")),
            ..all(&r, "gate-pass.json")
        };
        let c = &run(&o).unwrap().cells[4];
        assert_eq!(c.status, Status::Open, "{}", c.evidence);
        assert!(
            c.evidence.contains("built-ins/Math: 1 files not run"),
            "{}",
            c.evidence
        );
    }

    /// The JetStream hint names an ep9 config: run.mjs reads a bare path as a d8 engine, so
    /// an ep9 build needs `ep9@<path>` (M7-P0 reviewer R3).
    #[test]
    fn the_jetstream_hint_names_an_ep9_engine() {
        let cfg = JETSTREAM_CMD.split(" --config ").nth(1).unwrap();
        let engine = cfg.split_once('=').unwrap().1;
        assert!(engine.starts_with("ep9@"), "{cfg}");
    }

    #[test]
    fn item5_dirs_and_fastdiff() {
        let r = repo("t262");
        let base = all(&r, "gate-pass.json");
        // One more Function failure: 18 / 20 = 90% < 95%.
        let exec = fixture("exec.json").replace(SHA, &r.head).replace(
            "\"path\": \"test/built-ins/Function/f0.js\"",
            "\"path\": \"test/built-ins/Function/f0.js\"}, {\"path\": \"test/built-ins/Function/f1.js\", \"status\": \"fail\"",
        );
        write(&r.root, "inputs/exec2.json", &exec);
        let o = Options {
            exec: Some(r.root.join("inputs/exec2.json")),
            ..all(&r, "gate-pass.json")
        };
        let rep = run(&o).unwrap();
        assert_eq!(rep.cells[4].status, Status::Fail);
        assert!(
            rep.cells[4]
                .evidence
                .contains("built-ins/Function/ 18 / 20 = 90.00% (>= 95%) FAIL")
        );
        // A fast path with 0 cases is OPEN; a mismatch is FAIL.
        for (from, to, want) in [
            ("\"cases\": 4", "\"cases\": 0", Status::Open),
            (
                "\"cases\": 4, \"mismatches\": 0",
                "\"cases\": 4, \"mismatches\": 2",
                Status::Fail,
            ),
        ] {
            write(
                &r.root,
                "inputs/fd.json",
                &fixture("fastdiff.json")
                    .replace(SHA, &r.head)
                    .replace(from, to),
            );
            let o = Options {
                fastdiff: Some(r.root.join("inputs/fd.json")),
                ..all(&r, "gate-pass.json")
            };
            assert_eq!(run(&o).unwrap().cells[4].status, want, "{to}");
        }
        // A filtered run: fewer language files than the checkout has.
        write(
            &r.root,
            "inputs/exec3.json",
            &fixture("exec.json")
                .replace(SHA, &r.head)
                .replace("\"total\": 20", "\"total\": 19"),
        );
        let o = Options {
            exec: Some(r.root.join("inputs/exec3.json")),
            ..base
        };
        let c = &run(&o).unwrap().cells[4];
        assert_eq!(c.status, Status::Fail);
        assert!(
            c.evidence
                .contains("19 language files in the run, the checkout has 20"),
            "{}",
            c.evidence
        );
    }

    #[test]
    fn item4_a_cap_over_fails_and_few_runs_are_open() {
        let r = repo("mem");
        let over = fixture("mem.json")
            .replace(SHA, &r.head)
            .replace("110000000", "130000000");
        write(&r.root, "inputs/mem2.json", &over);
        let o = Options {
            mem: Some(r.root.join("inputs/mem2.json")),
            ..opts(&r, HOST)
        };
        let c = &run(&o).unwrap().cells[3];
        assert_eq!(c.status, Status::Fail);
        assert!(
            c.evidence
                .contains("ep9 jit splay: 1.300x node-jitless (cap 1.2)"),
            "{}",
            c.evidence
        );
        let few = fixture("mem.json").replace(SHA, &r.head).replacen(
            "\"n\": 5, \"median_bytes\": 45000000",
            "\"n\": 3, \"median_bytes\": 45000000",
            1,
        );
        write(&r.root, "inputs/mem3.json", &few);
        let o = Options {
            mem: Some(r.root.join("inputs/mem3.json")),
            ..opts(&r, HOST)
        };
        let c = &run(&o).unwrap().cells[3];
        assert_eq!(c.status, Status::Open);
        assert!(
            c.evidence.contains("3 runs (class Mem needs >= 5)"),
            "{}",
            c.evidence
        );
        // A failed load guard after the run, and a jitless engine whose build line names the
        // jit preset: each makes the cell OPEN.
        let busy = fixture("mem.json")
            .replace(SHA, &r.head)
            .replace(
                "\"after\": { \"load1\": 1.1, \"limit\": 1.5, \"pass\": true }",
                "\"after\": { \"load1\": 9, \"limit\": 1.5, \"pass\": false }",
            )
            .replace("features none; preset jitless", "features jit; preset jit");
        write(&r.root, "inputs/mem4.json", &busy);
        let o = Options {
            mem: Some(r.root.join("inputs/mem4.json")),
            ..opts(&r, HOST)
        };
        let c = &run(&o).unwrap().cells[3];
        assert_eq!(c.status, Status::Open);
        for want in [
            "the load guard did not pass before and after the run",
            "engine ep9-jitless: its build line \"features jit; preset jit\" does not name preset jitless",
        ] {
            assert!(c.evidence.contains(want), "{want}: {}", c.evidence);
        }
    }

    /// M7-P0 Codex r2 C2-2 part A: item 7 took its subjects by config name, so a d8 or node
    /// engine under the name `ep9` read PASS. Now a subject is a config of kind `ep9`, and its
    /// version holds the `ep9 --version` build line.
    #[test]
    fn item7_needs_an_ep9_engine() {
        let r = repo("js-kind");
        let o = Options {
            jetstream: Some(input(&r, "jetstream.json")),
            ..opts(&r, HOST)
        };
        let c = &run(&o).unwrap().cells[6];
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        assert!(
            c.evidence
                .contains("ep9: 3 of 3 workloads ran, geomean score ratio to jitless 0.800"),
            "{}",
            c.evidence
        );
        let ep9 = "\"kind\": \"ep9\", \"version\": \"ep9 0.0.0; features jit; preset jit\"";
        let base = fixture("jetstream.json").replace(SHA, &r.head);
        assert!(base.contains(ep9));
        for (name, to, want) in [
            (
                "d8",
                "\"kind\": \"d8\", \"version\": null",
                "no ep9 config (a config of kind \"ep9\"",
            ),
            (
                "no-version",
                "\"kind\": \"ep9\", \"version\": null",
                "ep9: the version has no ep9 build line (`features <list>; preset <name>`): null",
            ),
            (
                "no-build-line",
                "\"kind\": \"ep9\", \"version\": \"ep9 0.0.0\"",
                "ep9: the version has no ep9 build line (`features <list>; preset <name>`): ep9 0.0.0",
            ),
        ] {
            let f = format!("inputs/js-{name}.json");
            write(&r.root, &f, &base.replace(ep9, to));
            let o = Options {
                jetstream: Some(r.root.join(&f)),
                ..opts(&r, HOST)
            };
            let c = &run(&o).unwrap().cells[6];
            assert_eq!(c.status, Status::Open, "{name}: {}", c.evidence);
            assert!(c.evidence.contains(want), "{name}: {}", c.evidence);
        }
    }

    /// M7-P0 Codex r3 C3-1: run.mjs keys the table and the summary by config name. With a node
    /// --jitless config and an ep9 config of one name, and every ep9 run failed, the file holds
    /// only the node scores under that name (`<name>_n` = reps, complete). Item 7 read PASS,
    /// ratio 1.000, with 0 ep9 scores.
    #[test]
    fn item7_rejects_duplicate_config_names() {
        let r = repo("js-dup");
        let base = fixture("jetstream.json").replace(SHA, &r.head);
        for name in ["jitless", "ep9"] {
            let configs = base
                .replace(
                    "{ \"name\": \"jitless\", \"engine\"",
                    "{ \"name\": \"N\", \"engine\"",
                )
                .replace(
                    "{ \"name\": \"ep9\", \"engine\"",
                    "{ \"name\": \"N\", \"engine\"",
                );
            // The node scores only, under the shared name.
            let mut text = String::new();
            for line in configs.lines() {
                if line.starts_with("  \"ep9\": { \"complete\"") {
                    continue;
                }
                let line = line
                    .replace(", \"ep9\": 16, \"ep9_n\": 3 }", " }")
                    .replace("\"jitless\": 20, \"jitless_n\": 3", "\"N\": 20, \"N_n\": 3")
                    .replace("  \"jitless\": { \"complete\"", "  \"N\": { \"complete\"")
                    .replace("\"ratio_vs_first\": 1 },", "\"ratio_vs_first\": 1 }");
                text.push_str(&line.replace("\"N", &format!("\"{name}")));
                text.push('\n');
            }
            assert_eq!(text.matches(&format!("\"name\": \"{name}\"")).count(), 2);
            assert!(!text.contains(": 16"), "{text}");
            assert!(parse_json(&text).is_ok(), "{text}");
            let f = format!("inputs/js-dup-{name}.json");
            write(&r.root, &f, &text);
            let o = Options {
                jetstream: Some(r.root.join(&f)),
                ..opts(&r, HOST)
            };
            let c = &run(&o).unwrap().cells[6];
            assert_eq!(c.status, Status::Open, "{name}: {}", c.evidence);
            let want = format!("two configs have the name \"{name}\"");
            assert!(c.evidence.contains(&want), "{name}: {}", c.evidence);
        }
    }

    #[test]
    fn item7_short_workloads_and_item8_crashes() {
        let r = repo("js");
        let short = fixture("jetstream.json").replace(SHA, &r.head).replace(
            "\"ep9_n\": 3 },\n  { \"name\": \"Air\"",
            "\"ep9_n\": 2 },\n  { \"name\": \"Air\"",
        );
        write(&r.root, "inputs/js2.json", &short);
        let crash = fixture("fuzz-fuzzilli.json")
            .replace(SHA, &r.head)
            .replace("\"cpu_hours\": 260.5", "\"cpu_hours\": 520")
            .replace(
                "\"crashes\": []",
                "\"crashes\": [{\"id\": \"abc\", \"state\": \"open\"}]",
            );
        write(&r.root, "inputs/fz2.json", &crash);
        let o = Options {
            jetstream: Some(r.root.join("inputs/js2.json")),
            fuzz: vec![r.root.join("inputs/fz2.json")],
            ..opts(&r, HOST)
        };
        let rep = run(&o).unwrap();
        assert_eq!(rep.cells[6].status, Status::Open);
        assert!(
            rep.cells[6]
                .evidence
                .contains("1 of 3 workloads lack 3 scores"),
            "{}",
            rep.cells[6].evidence
        );
        assert_eq!(rep.cells[7].status, Status::Fail);
        assert!(
            rep.cells[7].evidence.contains("open unique crashes: abc"),
            "{}",
            rep.cells[7].evidence
        );
        // 520 h with no crash passes; a dirty record is not counted.
        write(
            &r.root,
            "inputs/fz3.json",
            &crash.replace("{\"id\": \"abc\", \"state\": \"open\"}", ""),
        );
        let o = Options {
            fuzz: vec![r.root.join("inputs/fz3.json")],
            ..opts(&r, HOST)
        };
        assert_eq!(run(&o).unwrap().cells[7].status, Status::Pass);
        write(
            &r.root,
            "inputs/fz4.json",
            &crash
                .replace("{\"id\": \"abc\", \"state\": \"open\"}", "")
                .replace("\"dirty\": false", "\"dirty\": true"),
        );
        let o = Options {
            fuzz: vec![r.root.join("inputs/fz4.json")],
            ..opts(&r, HOST)
        };
        assert_eq!(run(&o).unwrap().cells[7].status, Status::Open);
    }

    #[test]
    fn p_gate_rows_and_usage_errors() {
        let rows = p_gate_rows(&fixture("p-gates.md"));
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2], ("P4".to_string(), "tripped".to_string()));
        let a = |v: &[&str]| {
            parse_args(
                &v.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                PathBuf::from("."),
            )
        };
        assert!(a(&["--host", HOST]).is_err());
        assert!(
            a(&["--host", "x86_64-apple-darwin", "--out", "x"])
                .unwrap_err()
                .contains("not a tier-1 target")
        );
        assert!(a(&["--host", HOST, "--out", "x", "--bogus"]).is_err());
        let r = repo("usage");
        let o = Options {
            mem: Some(input(&r, "fastdiff.json")),
            ..opts(&r, HOST)
        };
        assert!(run(&o).unwrap_err().contains("not an ep9-mem/v1 file"));
        let o = Options {
            exec: Some(input(&r, "exec.json")),
            ..opts(&r, LINUX_X64)
        };
        assert!(run(&o).unwrap_err().contains("but --host is"));
    }

    #[test]
    fn build_lines() {
        assert!(build_line_check("jit", Some("features jit; preset jit")).is_ok());
        assert!(build_line_check("jit", Some("features jit,become; preset jit")).is_ok());
        assert!(build_line_check("jitless", Some("features none; preset jitless")).is_ok());
        assert!(build_line_check("jit", Some("features none; preset jit")).is_err());
        assert!(build_line_check("jitless", Some("features jit; preset jitless")).is_err());
        assert!(build_line_check("jit", None).is_err());
    }
}
