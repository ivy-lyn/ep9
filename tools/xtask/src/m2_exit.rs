//! `cargo xtask m2-exit` (M2 exit check, docs/plan/09-milestones.md section 2.2, M2 exit 1-10;
//! plan v12 GATES10 task 3): the table of the 10 M2 exit items and the D11 row, PASS, FAIL, OPEN,
//! N/A or NOT-RUN per item, for one host. Run it once per host: each run is one column.
//!
//! ```text
//! cargo xtask m2-exit --host <triple> --out m2-exit.md [--root <dir>]
//!     [--gate <json>]         tools/bench/cross/m2gate.mjs --out (ep9-m2gate/v1)   items 1, 2, 3, 6, 10, D11
//!     [--linux-gate <json>]   the m2gate JSON of the Linux x64 host (Windows host only: the ratio)
//!     [--pathb <json>]        the Path B 2-box compare (ep9-pathb-compare/v1)       items 1, 2, 3, 6, 10, D11
//!                             (Linux x64 and Windows x64 hosts only; V13-D37)
//!     [--linux-pathb <json>]  the Path B compare of the Linux x64 GitHub legs       items 1, 2, 3, 6, 10, D11
//!                             (Windows x64 host only: the same-CPU-model pairs, V15-D16)
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
//! | 4 | MEMREAD: per build and v8-v7 kernel, the median peak RSS of ep9 over node --jitless <= 1.2 and over d8 --jitless <= 1.8 (G7, 02-architecture.md), >= 5 runs per row (class Mem), the load guard passed. On `aarch64-unknown-linux-gnu` the d8 half is N/A by design (no linux-arm64 V8 in jsvu, DL:5351; plan v15 V15-D18): the node cap still gates |
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
//! `valid`, a gate verdict is not needed). The verdict is the cross-host ratio rule of
//! 10-quality.md section 3 (P0-WININT): the ratio interval [Windows low / Linux high, Windows
//! high / Linux low] passes when its low end is >= 0.9, fails when its high end is < 0.9, and is
//! OPEN (marginal) when it holds 0.9. The cell reads OPEN "marginal (no gate host)" until both
//! runs are gate-host runs on the same CPU model (a dual-boot or identical host, 10:157).
//!
//! **The host-cell rule** (plan v13 V13-D37; 10-quality.md section 3, the 2026-11-13 note). With
//! no gate-host reading, a Linux x64 or Windows x64 speed cell reads the Path B 2-box compare
//! (`--pathb`): per build and item, the two box intervals against the item's M2 threshold
//! (Octane-7 1.33, v8-v7 1.0, TS 1.0 (the Linux x64 rule), splay 1.0); "above" when the low end
//! is >= the threshold, "below" when the high end is under it. The cell reads one of:
//!
//! | Cell | When | Exit |
//! |---|---|---|
//! | `MARGINAL (no gate host): AGREE above` | every part: both boxes above | holds; read again on the gate host when one exists (a fail there reopens M2) |
//! | `FAIL-SIDE (no gate host): AGREE below` | a part: both boxes below | blocks the M2 exit |
//! | `MARGINAL: NO AGREEMENT` | else: a box straddles, the boxes disagree, a part has no cell, or the compare is not a valid reading of this tree | blocks the M2 exit |
//!
//! The status stays OPEN (a box never passes or fails a gate, 10:156); the cell names its form.
//! A gate-host reading (Linux x64: every run with `gate_host`; Windows x64: the gate-host pair
//! on one CPU model) decides as before, and Path B is not read. A valid compare names a full
//! `commit` that is fresh for this tree, the `platform` of the host (`linux-x64`,
//! `win32-x64`), and for each box (1, 2) and build of the cell a run check with `commit_ok`, at
//! least 31 `rounds` and the load guard passed before and after (10-quality.md section 3, the
//! box protocol). The verdict line follows V13-D37: the M2 exit holds on the host when every cell is
//! PASS or N/A, or the cells that are not read `MARGINAL (no gate host): AGREE above`.
//!
//! **The win-x64 same-CPU-model form** (plan v15 V15-D16). Windows x64 is compared with Linux x64
//! on the same hardware class (10-quality.md:84) and within 10% (09-milestones.md:66). A
//! GitHub Windows runner and a GitHub Linux x64 runner of one CPU model (the same `model name`)
//! are the same hardware class, but not a gate host (10:157). So on `x86_64-pc-windows-msvc`,
//! with no gate-host reading, `--pathb` (the windows compare) and `--linux-pathb` (the Linux x64
//! compare of the same dispatch), the cell reads the `legs` of both: each Windows leg is paired
//! only with each Linux x64 leg of the same CPU model. Per item and build, each pair gives the
//! ratio interval [Windows low / Linux high, Windows high / Linux low] of the two leg intervals
//! (for TS: 1 / the gross ep9 time `time_s` at each end, 10:157; not the ratio to node).
//! The part is above when every pair's low end is >= 0.9, below when every pair's high end is
//! < 0.9, else neither. The cell reads `MARGINAL (no gate host): AGREE above` when every part is
//! above, `FAIL-SIDE (no gate host): AGREE below` when a part is below, else `MARGINAL: NO
//! AGREEMENT` (also when a leg of a pair is not a valid run: a fresh full commit, one commit on
//! every leg, a clean binary, >= 31 rounds and the load guard passed in each build; when a leg of
//! a compare was not read, has no CPU model, or is missing or given twice; when the legs are of 2
//! GitHub runs or a host.toml slot id is not there once (no such id on any leg: a note only); and
//! when the windows compare has `legs` but no `--linux-pathb`: the pairs are then not known). With
//! no same-model pair, or an older compare (no `legs`), the cell keeps the form above (CPU models).
//!
//! **Fresh inputs.** A speed, MEMREAD, FASTDIFF or JetStream input names a full commit id that is
//! HEAD, or an ancestor with no change in the paths it measures since, and says that its tree
//! had no uncommitted change; else the cell is at least OPEN. An exec JSON may instead name the
//! git id of each path it measures (a CI pull_request job runs on GitHub's merge commit).
//!
//! **The fork checkouts** (FORK-CHECKOUT-ATTEST, `exit::forks`): the exec JSON names the fork
//! line of its runner's build (`"runner_forks"`): item 5 is OPEN unless every fork reads
//! `pinned` (a missing field too). An ep9 binary built from a fork checkout that is not its pin
//! plus its patches says `dirty` in its `ep9 --version` build line (`crates/ep9_cli/build.rs`),
//! so the speed, MEMREAD and JetStream inputs that copy that line read it as an uncommitted
//! change.
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
//!   and `mismatches`, and for a row with `off` `switch` its `hits_on` and `hits_off` (route
//!   P0-FASTHITS: the fast path ran with the fast paths on, never with them off).
//! - JetStream (jetstream/run.mjs): `list`, `jetstream_commit`, `reps`, `host.{platform, arch}`,
//!   `ep9.{commit, dirty}`, `configs[]` (`name`, `engine`, `kind`, `version`, `flags`) and
//!   `table[]` (per workload `<config>` median score and `<config>_n` scores). The ep9 configs are
//!   the ones of kind `ep9` (an `ep9` or `ep9@<path>` engine), and the version of each holds the
//!   build line of `ep9 --version`; node --jitless is the config of engine `node` (or kind
//!   `node`: a `node@<path>` engine) with the flag `--jitless`. Two configs of one name read
//!   OPEN (the table is keyed by name).
//! - P-gate file: a Markdown table with the columns `Gate` and `Result`.
//! - ep9-pathb-compare/v1 (the Path B compare of the 2 box runs): `commit`, `platform`,
//!   `checks[]` with `box`, `build`, `commit_ok`, `rounds` and `load.{before, after}.pass`, and
//!   `cells[]` with `build`, `cell` (the M2 item cells start `item <n>:`), `threshold`, `box1`
//!   and `box2` (`median`, `interval`) and `result`; and `legs[]` (V15-D16) with `leg`, `cpu`,
//!   `platform`, `commit`, `checks[]` (`build`, `commit_ok`, `rounds`, `load`) and `cells[]`
//!   (`build`, `cell`, `value.{median, interval}`).
//!
//! The extra rows (no exit-code effect): the parity debt of item 2 and the tripped P-gates.
//!
//! Exit 0 when the M2 exit holds on the host (every cell PASS or N/A, or by the host-cell rule),
//! 1 otherwise, 2 on a usage error or an input that cannot be read.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::exit::cross::fresh_since;
use crate::exit::forks;
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

/// What a speed input measured: the engine, its build and the forks it links: regress and
/// icu_calendar (ep9_temporal and ep9_intl build it from the git-ignored checkout of
/// `third_party/icu4x.commit` and `third_party/patches/icu4x`; M2EXIT-ICU4X-FRESH, the m1-exit
/// form of INTL-FRESH-PATHS).
const FRESH_PATHS: [&str; 9] = [
    "crates",
    "tools/bench",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "third_party/regress.commit",
    "third_party/patches/regress",
    "third_party/icu4x.commit",
    "third_party/patches/icu4x",
];

/// What a FASTDIFF run tested: the engine and its forks (no bench tool).
const ENGINE_PATHS: [&str; 8] = [
    "crates",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "third_party/regress.commit",
    "third_party/patches/regress",
    "third_party/icu4x.commit",
    "third_party/patches/icu4x",
];

/// What a test262 run read (as m1-exit's fresh, named inputs).
const T262_PATHS: [&str; 11] = [
    "crates",
    "tools/bench",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "tools/test262",
    "third_party/test262.commit",
    "third_party/regress.commit",
    "third_party/patches/regress",
    "third_party/icu4x.commit",
    "third_party/patches/icu4x",
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
const FUZZ_PATHS: [&str; 8] = ENGINE_PATHS;

const P_GATES: [&str; 3] = ["P1", "P2", "P4"];
const P_RESULTS: [&str; 2] = ["not tripped", "tripped"];

/// Windows x64 within 10% of Linux x64 (09:66).
const WINDOWS_MIN: f64 = 0.9;

/// The Path B compare (V13-D37): its schema, the paired rounds of the box protocol
/// (10-quality.md section 3, "Owner action": 31 paired rounds), and the M2 item and threshold
/// of each speed reading (09-milestones.md section 2.2; the TS rule of a Linux x64 host, V12-D5).
const PATHB_SCHEMA: &str = "ep9-pathb-compare/v1";
const PATHB_ROUNDS: f64 = 31.0;
const PATHB_ITEMS: [(&str, u32, f64); 4] = [
    (OCTANE, 1, 1.33),
    (V8V7, 2, 1.0),
    (TS, 3, 1.0),
    (SPLAY, 6, 1.0),
];
const PATHB_HOW: &str = "the Path B compare JSON (ep9-pathb-compare/v1) of the 2 box runs of this tree (plan section 13.2)";
const LINUX_PATHB_HOW: &str = "node .github/scripts/exit-inputs.mjs pathb --platform linux-x64 --leg <dir> (the pathb-l1, pathb-l2 artifacts of the exit-inputs dispatch) --out linux.json";

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
     [--gate <json>] [--linux-gate <json>] [--pathb <json>] [--linux-pathb <json>] [--mem <json>] [--exec <json>] \
     [--fastdiff <json>] [--jetstream <json>] [--fuzz <json>]... [--p-gates <md>]";

/// The parsed command line.
#[derive(Debug, Default)]
struct Options {
    root: PathBuf,
    host: String,
    out: PathBuf,
    gate: Option<PathBuf>,
    linux_gate: Option<PathBuf>,
    pathb: Option<PathBuf>,
    linux_pathb: Option<PathBuf>,
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
            "--pathb" => o.pathb = Some(PathBuf::from(val()?)),
            "--linux-pathb" => o.linux_pathb = Some(PathBuf::from(val()?)),
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
    if o.linux_pathb.is_some() && o.host != WINDOWS_X64 {
        return Err(format!(
            "--linux-pathb is for --host {WINDOWS_X64} only (V15-D16)\n{USAGE}"
        ));
    }
    if o.pathb.is_some() && o.host != LINUX_X64 && o.host != WINDOWS_X64 {
        return Err(format!(
            "--pathb is for --host {LINUX_X64} or {WINDOWS_X64} only (V13-D37)\n{USAGE}"
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
    Ok(exit_holds(&report.cells, &report.forms))
}

/// The rendered result.
#[derive(Debug)]
struct Report {
    table: String,
    markdown: String,
    /// Items 1-10, then D11.
    cells: Vec<Cell>,
    /// The host-cell form of each cell (V13-D37), or None.
    forms: Vec<Option<HostForm>>,
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
    pathb: Option<(PathBuf, Json)>,
    linux_pathb: Option<(PathBuf, Json)>,
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
        pathb: one(&o.pathb)?,
        linux_pathb: one(&o.linux_pathb)?,
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
    schema("--pathb", &c.pathb, PATHB_SCHEMA)?;
    schema("--linux-pathb", &c.linux_pathb, PATHB_SCHEMA)?;
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
    if o.host == LINUX_X64 || o.host == WINDOWS_X64 {
        miss(
            c.pathb.is_some() || on_gate_host(&c, &BUILDS),
            "--pathb",
            "items 1, 2, 3, 6, 10, D11 with no gate host (V13-D37)",
            PATHB_HOW,
        );
    }
    if o.host == WINDOWS_X64 {
        miss(
            c.linux_pathb.is_some() || on_gate_host(&c, &BUILDS),
            "--linux-pathb",
            "the same-CPU-model form of items 1, 2, 3, 6, 10, D11 with no gate host (V15-D16)",
            LINUX_PATHB_HOW,
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
        host_cell(&c, &[OCTANE], &BUILDS),
        host_cell(&c, &[V8V7], &BUILDS),
        host_cell(&c, &[TS], &BUILDS),
        (item4(&c), None),
        (item5(&c), None),
        host_cell(&c, &[SPLAY], &BUILDS),
        (item7(&c), None),
        (item8(&c), None),
        (item9(&c), None),
        host_cell(&c, &[OCTANE, V8V7, TS], &["jit"]),
        host_cell(&c, &[OCTANE, V8V7, TS], &["jitless"]),
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

/// The build-id OPEN rule (route P0-BUILDID): the bench tools give a run the commit of the ep9
/// binary they measured (its `ep9 --version` build line on stderr, `crates/ep9_cli/build.rs`),
/// and null when a binary prints no line, names no commit, was built with uncommitted engine
/// changes, or two binaries name two commits; [`fresh`] reads that null as OPEN. This adds
/// the reason from the run's record of the lines (`commit.binary` of mem.mjs, `ep9.binary` of
/// jetstream/run.mjs: one record or a list; m2gate.mjs carries none). It trusts that record
/// as the tool wrote it; a JSON without it (a tool before P0-BUILDID) keeps the plain reason.
fn with_binary_why(why: String, binary: Option<&Json>) -> String {
    let one = |b: &Json| -> Option<&'static str> {
        match b {
            Json::Null => Some("an ep9 binary prints no build line (`ep9 --version`, P0-BUILDID)"),
            Json::Obj(_) if b.get("dirty").and_then(Json::bool) == Some(true) => {
                Some("an ep9 binary was built with uncommitted engine changes")
            }
            Json::Obj(_) if b.get("commit").and_then(Json::str).is_none() => {
                Some("the ep9 binaries name no one commit")
            }
            _ => None,
        }
    };
    let found = match binary {
        Some(Json::Arr(a)) => a.iter().find_map(one).or_else(|| {
            let mut c = a.iter().filter_map(|b| b.get("commit").and_then(Json::str));
            let first = c.next();
            c.any(|x| Some(x) != first)
                .then_some("the ep9 binaries name no one commit")
        }),
        Some(b) => one(b),
        None => None,
    };
    match found {
        Some(f) => format!("{why} ({f})"),
        None => why,
    }
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

/// The order-statistic interval of [`speed`] (low, high): a ratio reading's `interval`, or
/// for `ts` 1 / its time interval's ends (P0-WININT). None if a bound is missing or not > 0.
fn speed_interval(id: &str, r: &Json) -> Option<(f64, f64)> {
    let iv = r.get("interval").and_then(Json::arr)?;
    let (a, z) = (iv.first()?.num()?, iv.get(1)?.num()?);
    if !(a > 0.0 && z >= a) {
        return None;
    }
    Some(if id == TS { (1.0 / z, 1.0 / a) } else { (a, z) })
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
            // P0-WININT: the cross-host ratio rule (10-quality.md section 3, the Windows note):
            // the ratio interval [Windows low / Linux high, Windows high / Linux low] of the two
            // speed intervals; the median ratio is printed, the interval decides.
            let wi = reading(win, id, b).and_then(|r| speed_interval(id, r));
            let li = lr.and_then(|r| speed_interval(id, r));
            match (w, l, wi, li) {
                (Some(w), Some(l), Some((wl, wh)), Some((ll, lh))) if l > 0.0 => {
                    let q = w / l;
                    let (lo, hi) = (wl / lh, wh / ll);
                    let side = if lo >= WINDOWS_MIN {
                        ""
                    } else if hi < WINDOWS_MIN {
                        status = status.max(Status::Fail);
                        " below"
                    } else {
                        status = status.max(Status::Open);
                        " marginal (the interval holds 0.9: run both hosts again with 2x rounds)"
                    };
                    ratios.push(format!(
                        "{b} {id} Windows / Linux x64 = {q:.3} [{lo:.3}, {hi:.3}]{side}"
                    ));
                }
                (Some(_), Some(l), _, _) if l > 0.0 => {
                    status = status.max(Status::Open);
                    ratios.push(format!("{b} {id}: no interval on both hosts"));
                }
                _ => {
                    status = status.max(Status::Open);
                    ratios.push(format!("{b} {id}: no value on both hosts"));
                }
            }
        }
    }
    let text = format!(
        "ratio to Linux x64 ({}; the interval >= {WINDOWS_MIN} needed, 09:66): {}",
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
                if cmp == "d8-jitless" && c.o.host == LINUX_A64 {
                    // V15-D18: no linux-arm64 V8 in jsvu, so no d8 rows; the node cap gates.
                    continue;
                }
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
    let d8_half = if c.o.host == LINUX_A64 {
        "; the d8 --jitless half: N/A by design (no linux-arm64 V8 in jsvu, DL:5351; V15-D18)"
    } else {
        ""
    };
    with_fresh(
        cell(
            status,
            format!(
                "{}: median peak RSS of ep9 per v8-v7 kernel; {}{d8_half}{}",
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
        )
        .map_err(|e| with_binary_why(e, j.path(&["commit", "binary"]))),
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
    // FORK-CHECKOUT-ATTEST: the runner was built from fork checkouts that were their pins plus
    // their patches (`"runner_forks"`, exit::forks).
    let src = src.and_then(|t| {
        forks::check(j.get("runner_forks").and_then(Json::str), "runner_forks").map(|()| t)
    });
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
        // P0-FASTHITS: 0 mismatches from cases that never took a Switch fast path is no
        // evidence; a hit with the fast paths off means the switch did not turn it off.
        if f.get("off").and_then(Json::str) == Some("switch") {
            match (num(f, "hits_on"), num(f, "hits_off")) {
                (Some(on), Some(off)) => {
                    if on < 1.0 {
                        status = Status::Fail;
                        why.push(format!("{name}: 0 fast path hits"));
                    }
                    if off != 0.0 {
                        status = Status::Fail;
                        why.push(format!("{name}: {off} hits with the fast paths off"));
                    }
                }
                _ => {
                    status = status.max(Status::Open);
                    why.push(format!("{name}: no hit count"));
                }
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
        )
        .map_err(|e| with_binary_why(e, j.path(&["ep9", "binary"]))),
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
    cells: Vec<(Cell, Option<HostForm>)>,
    extra: Vec<(&'static str, Cell)>,
    missing: Vec<String>,
) -> Report {
    let o = c.o;
    let (cells, forms): (Vec<Cell>, Vec<Option<HostForm>>) = cells.into_iter().unzip();
    // The cell text: its host-cell form (V13-D37), else its status.
    let name = |h: &Cell, f: &Option<HostForm>| f.map_or(h.status.name(), HostForm::label);
    let head = git(&o.root, &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let mut table = String::new();
    let _ = writeln!(table, "| # | M2 exit item | {} |", o.host);
    let _ = writeln!(table, "|---|---|---|");
    for (((l, t), h), f) in LABELS.iter().zip(TITLES).zip(&cells).zip(&forms) {
        let _ = writeln!(table, "| {l} | {t} | {} |", name(h, f));
    }
    let all_pass = cells.iter().all(|c| c.status.ok());
    // "item 3 reads" or "items 3, 10 read".
    let items = |want: HostForm| {
        let l: Vec<&str> = LABELS
            .iter()
            .zip(&forms)
            .filter(|(_, f)| **f == Some(want))
            .map(|(l, _)| *l)
            .collect();
        match l.as_slice() {
            [one] => format!("item {one} reads"),
            _ => format!("items {} read", l.join(", ")),
        }
    };
    let (above, below) = (items(HostForm::AgreeAbove), items(HostForm::AgreeBelow));
    // V13-D37: a MARGINAL (no gate host) cell with a Path B AGREE above counts for the exit.
    let holds = exit_holds(&cells, &forms);
    let mut md = String::from("# M2 exit check (`cargo xtask m2-exit`)\n\n");
    let _ = writeln!(
        md,
        "- Items: docs/plan/09-milestones.md section 2.2, M2 exit 1-10, and the D11 row (11-decisions.md D11; plan v12 V12-D6)."
    );
    let _ = writeln!(md, "- Host column: {}; tree at commit `{head}`.", o.host);
    let result = if all_pass {
        "every item PASS (N/A cells are by design): the M2 exit holds on this host".to_string()
    } else if holds {
        format!(
            "the M2 exit holds on this host by the host-cell rule (V13-D37): every other item PASS or N/A, and {above} {} (a Path B 2-box agreement on the pass side; each is read again on the gate host when one exists, and a fail there reopens M2)",
            HostForm::AgreeAbove.label()
        )
    } else if forms.contains(&Some(HostForm::AgreeBelow)) {
        format!(
            "not every cell is PASS; {below} {}: a Path B 2-box agreement below the threshold blocks the M2 exit (V13-D37)",
            HostForm::AgreeBelow.label()
        )
    } else {
        "not every cell is PASS".to_string()
    };
    let _ = writeln!(md, "- Result: {result}.\n");
    md.push_str(&table);
    md.push_str("\n## Evidence\n");
    for (((l, t), h), f) in LABELS.iter().zip(TITLES).zip(&cells).zip(&forms) {
        // A host-cell evidence starts with its form already.
        let _ = match f {
            Some(_) => write!(md, "\n### {l}. {t}\n\n- {} {}\n", o.host, h.evidence),
            None => write!(
                md,
                "\n### {l}. {t}\n\n- {} {}: {}\n",
                o.host,
                h.status.name(),
                h.evidence
            ),
        };
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
        forms,
        extra,
        missing,
    }
}

// ---- the host-cell rule (V13-D37): Linux x64 and Windows x64 with no gate host ------------

/// The M2 exit holds on this host (the verdict line and the exit code; V13-D37): every cell is
/// PASS or N/A, or reads `MARGINAL (no gate host): AGREE above`.
fn exit_holds(cells: &[Cell], forms: &[Option<HostForm>]) -> bool {
    cells
        .iter()
        .zip(forms)
        .all(|(h, f)| h.status.ok() || *f == Some(HostForm::AgreeAbove))
}

/// The form of a Linux x64 or Windows x64 speed cell that reads the Path B compare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostForm {
    /// Every part: both box intervals on the pass side. Holds for the exit (V13-D37).
    AgreeAbove,
    /// A part: both box intervals below the threshold. Blocks the exit.
    AgreeBelow,
    /// Anything else. Blocks the exit.
    NoAgreement,
}

impl HostForm {
    fn label(self) -> &'static str {
        match self {
            HostForm::AgreeAbove => "MARGINAL (no gate host): AGREE above",
            HostForm::AgreeBelow => "FAIL-SIDE (no gate host): AGREE below",
            HostForm::NoAgreement => "MARGINAL: NO AGREEMENT",
        }
    }
}

/// The gate-host flag of the `build` run of an m2gate JSON.
fn gate_host_run(j: &Json, build: &str) -> bool {
    build_block(j, build)
        .and_then(|b| b.path(&["host", "gate_host"]))
        .and_then(Json::bool)
        == Some(true)
}

/// The speed readings of `builds` come from a gate host: on Linux x64 every `--gate` run; on
/// Windows x64 every run of both hosts, on one CPU model (10-quality.md section 3, 10:157).
fn on_gate_host(c: &Ctx, builds: &[&str]) -> bool {
    let Some((_, g)) = &c.gate else {
        return false;
    };
    match c.o.host.as_str() {
        LINUX_X64 => builds.iter().all(|b| gate_host_run(g, b)),
        WINDOWS_X64 => {
            let Some((_, l)) = &c.linux_gate else {
                return false;
            };
            let cpu = |j: &Json, b: &str| {
                build_block(j, b)
                    .and_then(|x| x.path(&["host", "cpu"]))
                    .and_then(Json::str)
                    .map(str::to_string)
            };
            builds.iter().all(|b| {
                gate_host_run(g, b)
                    && gate_host_run(l, b)
                    && cpu(g, b).is_some()
                    && cpu(g, b) == cpu(l, b)
            })
        }
        _ => false,
    }
}

/// A speed cell (items 1, 2, 3, 6, 10, D11) with the host-cell rule: on Linux x64 and Windows
/// x64 with no gate-host reading and a `--pathb` input, the Path B form decides the cell text;
/// else [`speed_cell`] as it is.
fn host_cell(c: &Ctx, ids: &[&str], builds: &[&str]) -> (Cell, Option<HostForm>) {
    let base = speed_cell(c, ids, builds);
    let host = c.o.host.as_str();
    let Some((p, j)) = &c.pathb else {
        return (base, None);
    };
    if (host != LINUX_X64 && host != WINDOWS_X64) || on_gate_host(c, builds) {
        return (base, None);
    }
    let (form, text) = if host == WINDOWS_X64 {
        match cross_form(c, ids, builds) {
            Ok(x) => x,
            Err(none) => {
                let (f, t) = pathb_form(c, p, j, ids, builds);
                (
                    f,
                    format!("{none}, so the cell keeps the Path B form (V15-D16): {t}"),
                )
            }
        }
    } else {
        pathb_form(c, p, j, ids, builds)
    };
    let gate = if c.gate.is_some() {
        format!(
            "; the --gate reading ({}): {}",
            base.status.name(),
            base.evidence
        )
    } else {
        String::new()
    };
    (
        cell(Status::Open, format!("{}: {text}{gate}", form.label())),
        Some(form),
    )
}

/// One side of a box interval against `t`: Some(true) above (low end >= t), Some(false) below
/// (high end < t), None when it straddles or is not an interval.
fn pathb_side(b: Option<&Json>, t: f64) -> (Option<bool>, String) {
    let iv = b.and_then(|b| b.get("interval")).and_then(Json::arr);
    let (lo, hi) = match iv {
        Some([a, z]) => match (a.num(), z.num()) {
            (Some(a), Some(z)) if a <= z => (a, z),
            _ => return (None, "no interval".into()),
        },
        _ => return (None, "no interval".into()),
    };
    let med = b
        .and_then(|b| num(b, "median"))
        .map_or("-".to_string(), |m| format!("{m:.3}"));
    let side = if lo >= t {
        Some(true)
    } else if hi < t {
        Some(false)
    } else {
        None
    };
    let word = match side {
        Some(true) => "above",
        Some(false) => "below",
        None => "straddles",
    };
    (side, format!("{med} [{lo:.3}-{hi:.3}] {word}"))
}

/// The Path B form of the readings `ids` of `builds`, and the evidence text.
fn pathb_form(c: &Ctx, p: &Path, j: &Json, ids: &[&str], builds: &[&str]) -> (HostForm, String) {
    let mut why = Vec::new();
    // The compare must be a valid reading of this tree on this host's platform.
    let want = platform_of(&c.o.host).unwrap_or("");
    match j.get("platform").and_then(Json::str) {
        Some(got) if got == want => {}
        got => why.push(format!(
            "the compare names the platform {}, not {want}",
            got.unwrap_or("none")
        )),
    }
    let checks = j.get("checks").and_then(Json::arr).unwrap_or(&[]);
    let mut clean = true;
    for &b in builds {
        for n in [1.0, 2.0] {
            let run = checks.iter().find(|x| {
                num(x, "box") == Some(n) && x.get("build").and_then(Json::str) == Some(b)
            });
            let Some(run) = run else {
                why.push(format!("no run check of box {n} build {b}"));
                clean = false;
                continue;
            };
            if run.get("commit_ok").and_then(Json::bool) != Some(true) {
                clean = false;
                why.push(format!(
                    "box {n} {b}: the run is not of the compare commit, or its binary is dirty"
                ));
            }
            if num(run, "rounds").is_none_or(|r| r < PATHB_ROUNDS) {
                why.push(format!(
                    "box {n} {b}: fewer than {PATHB_ROUNDS} paired rounds (the box protocol)"
                ));
            }
            for when in ["before", "after"] {
                if run.path(&["load", when, "pass"]).and_then(Json::bool) != Some(true) {
                    why.push(format!(
                        "box {n} {b}: the load guard {when} the run did not pass"
                    ));
                }
            }
        }
    }
    let dirty = Json::Bool(!clean);
    let commit = j.get("commit").and_then(Json::str);
    let fresh = match fresh(&c.o.root, commit, Some(&dirty), &FRESH_PATHS) {
        Ok(t) => t,
        Err(e) => {
            why.push(format!("the compare is stale: {e}"));
            String::new()
        }
    };
    let valid = why.is_empty();
    let cells = j.get("cells").and_then(Json::arr).unwrap_or(&[]);
    let mut parts = Vec::new();
    let (mut all_above, mut any_below) = (true, false);
    for &b in builds {
        for &id in ids {
            let Some(&(_, item, t)) = PATHB_ITEMS.iter().find(|(i, _, _)| *i == id) else {
                continue;
            };
            let prefix = format!("item {item}:");
            let found: Vec<&Json> = cells
                .iter()
                .filter(|x| {
                    x.get("build").and_then(Json::str) == Some(b)
                        && x.get("cell")
                            .and_then(Json::str)
                            .is_some_and(|l| l.starts_with(&prefix))
                })
                .collect();
            let [x] = found.as_slice() else {
                all_above = false;
                parts.push(format!(
                    "{b} item {item}: {} cells (one needed)",
                    found.len()
                ));
                continue;
            };
            if num(x, "threshold").is_none_or(|v| (v - t).abs() > 1e-9) {
                all_above = false;
                parts.push(format!(
                    "{b} item {item}: the compare threshold is not the M2 threshold {t}"
                ));
                continue;
            }
            let (s1, t1) = pathb_side(x.get("box1"), t);
            let (s2, t2) = pathb_side(x.get("box2"), t);
            let agree = match (s1, s2) {
                (Some(true), Some(true)) => "AGREE above",
                (Some(false), Some(false)) => "AGREE below",
                (Some(a), Some(z)) if a != z => "OPPOSITE",
                _ => "NO AGREEMENT",
            };
            // The producer's own result must say the same (a second reader of one rule).
            let stated = x.get("result").and_then(Json::str);
            let same = stated == Some(agree);
            all_above &= agree == "AGREE above" && same;
            any_below |= agree == "AGREE below" && same;
            parts.push(format!(
                "{b} item {item} (threshold {t}): box 1 {t1}, box 2 {t2}: {agree}{}",
                if same {
                    String::new()
                } else {
                    format!(" (the compare says {})", stated.unwrap_or("nothing"))
                }
            ));
        }
    }
    let form = if !valid {
        HostForm::NoAgreement
    } else if any_below {
        HostForm::AgreeBelow
    } else if all_above {
        HostForm::AgreeAbove
    } else {
        HostForm::NoAgreement
    };
    let at = commit.map_or("no commit".to_string(), short);
    let fresh = if fresh.is_empty() {
        String::new()
    } else {
        format!("; fresh: {fresh}")
    };
    (
        form,
        format!(
            "Path B {} (commit {at}, 2 boxes, provisional by rule 10:156): {}{fresh}{}",
            p.display(),
            parts.join("; "),
            if valid {
                String::new()
            } else {
                format!("; not a valid reading of this tree: {}", why.join("; "))
            }
        ),
    )
}

// ---- the win-x64 same-CPU-model form (plan v15 V15-D16) -----------------------------------

/// The `legs` of a Path B compare that name `platform` and a CPU model.
fn compare_legs<'a>(j: &'a Json, platform: &str) -> Vec<&'a Json> {
    j.get("legs")
        .and_then(Json::arr)
        .unwrap_or(&[])
        .iter()
        .filter(|l| l.get("platform").and_then(Json::str) == Some(platform) && leg_cpu(l).is_some())
        .collect()
}

/// The CPU model of a leg: None when it is missing, empty, or the host.mjs `cpuModel` sentinel
/// `unknown` (the detection failed: no model is known, so the leg is in no pair).
fn leg_cpu(l: &Json) -> Option<&str> {
    l.get("cpu")
        .and_then(Json::str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "unknown")
}

fn leg_name(l: &Json) -> &str {
    l.get("leg").and_then(Json::str).unwrap_or("?")
}

/// The legs of a Path B compare of `platform` that were not read (a `runners` row with an
/// `error`) or that have no CPU model or no platform in `legs`, each as `<leg>: <why>`. A
/// runner of another known platform (not read by `--platform`) is not of this set.
fn unread_legs(j: &Json, platform: &str) -> Vec<String> {
    let of = |x: &Json| {
        x.get("platform")
            .and_then(Json::str)
            .is_none_or(|p| p == platform)
    };
    let runners = j.get("runners").and_then(Json::arr).unwrap_or(&[]);
    let legs = j.get("legs").and_then(Json::arr).unwrap_or(&[]);
    runners
        .iter()
        .filter(|r| of(r))
        .filter_map(|r| {
            let e = r.get("error").and_then(Json::str)?;
            Some(format!("{}: {e}", leg_name(r)))
        })
        .chain(legs.iter().filter(|l| of(l)).filter_map(|l| {
            if l.get("platform").and_then(Json::str).is_none() {
                Some(format!("{}: no platform", leg_name(l)))
            } else if leg_cpu(l).is_none() {
                Some(format!("{}: no CPU model", leg_name(l)))
            } else {
                None
            }
        }))
        .collect()
}

/// The CPU models of `legs`, each once, or "none".
fn models(legs: &[&Json]) -> String {
    let m: BTreeSet<&str> = legs.iter().filter_map(|l| leg_cpu(l)).collect();
    if m.is_empty() {
        "none".into()
    } else {
        m.into_iter().collect::<Vec<_>>().join(", ")
    }
}

/// The run checks of one leg for `builds` (the box protocol, as in [`pathb_form`]): the reasons
/// why the leg is not a valid run of this tree. A clean leg adds its commit to `commits`.
fn leg_why<'a>(
    c: &Ctx,
    leg: &'a Json,
    builds: &[&str],
    commits: &mut BTreeSet<&'a str>,
) -> Vec<String> {
    let name = leg_name(leg);
    let mut why = Vec::new();
    let checks = leg.get("checks").and_then(Json::arr).unwrap_or(&[]);
    let mut clean = true;
    for &b in builds {
        let Some(run) = checks
            .iter()
            .find(|x| x.get("build").and_then(Json::str) == Some(b))
        else {
            clean = false;
            why.push(format!("leg {name}: no {b} run check"));
            continue;
        };
        if run.get("commit_ok").and_then(Json::bool) != Some(true) {
            clean = false;
            why.push(format!(
                "leg {name} {b}: the run is not of the leg commit, or its binary is dirty"
            ));
        }
        if num(run, "rounds").is_none_or(|r| r < PATHB_ROUNDS) {
            why.push(format!(
                "leg {name} {b}: fewer than {PATHB_ROUNDS} paired rounds (the box protocol)"
            ));
        }
        for when in ["before", "after"] {
            if run.path(&["load", when, "pass"]).and_then(Json::bool) != Some(true) {
                why.push(format!(
                    "leg {name} {b}: the load guard {when} the run did not pass"
                ));
            }
        }
    }
    let commit = leg.get("commit").and_then(Json::str);
    if let Err(e) = fresh(&c.o.root, commit, Some(&Json::Bool(!clean)), &FRESH_PATHS) {
        why.push(format!("leg {name} is stale: {e}"));
    } else if let Some(x) = commit {
        commits.insert(x);
    }
    why
}

/// The speed interval (low, high) of the `build` cell of item `item` of a leg; Err: why not.
/// Items 1, 2, 6: the ratio `value.interval`. Item 3: 1 / the gross ep9 TS time `time_s.interval`
/// at each end (10-quality.md section 3, P0-WININT, as [`speed_interval`]); the ratio to node
/// --jitless of each host is not the TS speed (EI2 Codex r1 F2).
fn leg_interval(leg: &Json, build: &str, item: u32) -> Result<(f64, f64), String> {
    let prefix = format!("item {item}:");
    let found: Vec<&Json> = leg
        .get("cells")
        .and_then(Json::arr)
        .unwrap_or(&[])
        .iter()
        .filter(|x| {
            x.get("build").and_then(Json::str) == Some(build)
                && x.get("cell")
                    .and_then(Json::str)
                    .is_some_and(|l| l.starts_with(&prefix))
        })
        .collect();
    let [x] = found.as_slice() else {
        return Err(format!("{} cells (one needed)", found.len()));
    };
    let ts = item == 3;
    let key = if ts { "time_s" } else { "value" };
    match x.path(&[key, "interval"]).and_then(Json::arr) {
        Some([a, z]) => match (a.num(), z.num()) {
            (Some(a), Some(z)) if a > 0.0 && z >= a => {
                Ok(if ts { (1.0 / z, 1.0 / a) } else { (a, z) })
            }
            _ => Err(format!("no {key} interval")),
        },
        _ => Err(format!("no {key} interval")),
    }
}

/// An interval (low, high) of a leg value.
type Span = (f64, f64);

/// One part (an item in a build) over the same-model pairs: each pair's ratio interval
/// [Windows low / Linux high, Windows high / Linux low]. Some(true) when every low end is
/// >= 0.9, Some(false) when every high end is < 0.9, else None (also with no pair).
fn cross_side(pairs: &[(Span, Span)]) -> Option<bool> {
    if pairs.is_empty() {
        return None;
    }
    let iv: Vec<(f64, f64)> = pairs
        .iter()
        .map(|&((wl, wh), (ll, lh))| (wl / lh, wh / ll))
        .collect();
    if iv.iter().all(|&(lo, _)| lo >= WINDOWS_MIN) {
        Some(true)
    } else if iv.iter().all(|&(_, hi)| hi < WINDOWS_MIN) {
        Some(false)
    } else {
        None
    }
}

/// The pathb legs of the exit-inputs matrix (ci.yml): pathb-w1..w4 and pathb-l1, pathb-l2. The
/// same-model pairs are known only when each compare holds each of them once (EI2 Codex r2 F1).
const MATRIX_LEGS: [(&str, usize); 2] = [("win32-x64", 4), ("linux-x64", 2)];

/// The host.toml id of each pathb leg of the exit-inputs matrix, per platform (ci.yml: the host
/// step's `--id "gha-$LEG"`). Each compare must hold each of its ids once (EI2-R3-LEG-SLOT).
const MATRIX_IDS: [(&str, &[&str]); 2] = [
    (
        "win32-x64",
        &[
            "gha-pathb-w1",
            "gha-pathb-w2",
            "gha-pathb-w3",
            "gha-pathb-w4",
        ],
    ),
    ("linux-x64", &["gha-pathb-l1", "gha-pathb-l2"]),
];

/// A provenance field of a leg (`id`, `run_id`, `run_attempt`: the leg's host.toml, copied by
/// exit-inputs.mjs readLeg): None when it is missing or null; else its text (a string, or a
/// whole number), and "" for any other value.
fn leg_field(l: &Json, key: &str) -> Option<String> {
    match l.get(key)? {
        Json::Null => None,
        Json::Str(s) => Some(s.trim().to_string()),
        Json::Num(n) if n.is_finite() && n.fract() == 0.0 => Some(format!("{n:.0}")),
        _ => Some(String::new()),
    }
}

/// The provenance checks of the legs of both compares (`sets`: each platform and its legs).
/// EI2-R3-LEG-SLOT: each compare holds each host.toml id of [`MATRIX_IDS`] once. EI2-R1: every
/// leg names the same GitHub `run_id` (one dispatch). When no leg names an id (or no leg names
/// a run id), the input is older than these fields: that check gives a note only. Ok: the text
/// of the checks; Err: each reason (the cell then reads NO AGREEMENT).
fn provenance(sets: &[(&str, &[&Json])]) -> Result<String, Vec<String>> {
    let mut why = Vec::new();
    let mut text = Vec::new();
    let legs: Vec<&Json> = sets.iter().flat_map(|(_, l)| l.iter().copied()).collect();
    if legs.iter().all(|l| leg_field(l, "id").is_none()) {
        text.push(
            "note: no leg names its host.toml id (an older input), so the slots are not checked"
                .to_string(),
        );
    } else {
        let before = why.len();
        for &(p, ls) in sets {
            let want = MATRIX_IDS
                .iter()
                .find(|(q, _)| *q == p)
                .map_or(&[][..], |(_, w)| *w);
            let mut by: BTreeMap<String, Vec<&str>> = BTreeMap::new();
            for l in ls {
                match leg_field(l, "id") {
                    Some(id) => by.entry(id).or_default().push(leg_name(l)),
                    None => why.push(format!("{p} leg {}: no host.toml id", leg_name(l))),
                }
            }
            for w in want.iter().filter(|w| !by.contains_key(**w)) {
                why.push(format!("{p}: no leg has the id {w}"));
            }
            for (id, names) in &by {
                if !want.contains(&id.as_str()) {
                    why.push(format!(
                        "{p} leg {}: the id {id:?} is not a pathb slot of the matrix",
                        names.join(", ")
                    ));
                } else if names.len() > 1 {
                    why.push(format!(
                        "{p}: the id {id} is given {} times (legs {})",
                        names.len(),
                        names.join(", ")
                    ));
                }
            }
        }
        if why.len() == before {
            text.push("each pathb slot once (host.toml ids)".to_string());
        }
    }
    let mut runs: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut attempts = BTreeSet::new();
    let mut without = Vec::new();
    for l in &legs {
        match leg_field(l, "run_id") {
            Some(r) if r.is_empty() => why.push(format!(
                "leg {}: the run_id is empty or not a string",
                leg_name(l)
            )),
            Some(r) => {
                runs.entry(r).or_default().push(leg_name(l));
                attempts.insert(leg_field(l, "run_attempt").unwrap_or_else(|| "?".into()));
            }
            None => without.push(leg_name(l)),
        }
    }
    if runs.is_empty() && without.len() == legs.len() {
        text.push(
            "note: no leg names a GitHub run id (an older input), so one dispatch is not checked"
                .to_string(),
        );
    } else if !without.is_empty() {
        why.push(format!(
            "some legs name a GitHub run id and some do not (no run id: {})",
            without.join(", ")
        ));
    }
    if runs.len() > 1 {
        why.push(format!(
            "the legs are of {} GitHub runs, not one ({})",
            runs.len(),
            runs.iter()
                .map(|(r, n)| format!("run {r}: {}", n.join(", ")))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    } else if let Some(r) = runs.keys().next() {
        text.push(format!(
            "one GitHub run {r} (attempt {})",
            attempts.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    if why.is_empty() {
        Ok(text.join("; "))
    } else {
        Err(why)
    }
}

/// The win-x64 same-CPU-model form of the readings `ids` of `builds` (V15-D16, see the module
/// doc): the form and the evidence text. Err (the reason) when there is no same-model pair, or
/// when an older compare (no `legs`) has no `--linux-pathb`: the caller then reads the Path B
/// form.
fn cross_form(c: &Ctx, ids: &[&str], builds: &[&str]) -> Result<(HostForm, String), String> {
    let Some((wp, wj)) = &c.pathb else {
        return Err("no --pathb input".into());
    };
    let Some((lp, lj)) = &c.linux_pathb else {
        // Option B (EI2-R3-LINUX-PATHB-REQUIRED): a compare with a `legs` block is of an
        // exit-inputs dispatch, which also has Linux x64 legs. With no Linux x64 compare the
        // same-model pairs are not known: no verdict and no fallback. Only an older compare (no
        // `legs`: 2 boxes, no Linux x64 legs in its run) keeps the Path B form.
        if wj.get("legs").is_some() {
            return Ok((
                HostForm::NoAgreement,
                format!(
                    "Windows / Linux x64 same-CPU-model form (V15-D16; {}): the windows compare has a legs block (an exit-inputs dispatch); the Linux x64 compare is missing (pass --linux-pathb with the Linux x64 compare of the same dispatch), so the same-model pairs are not known",
                    wp.display()
                ),
            ));
        }
        return Err(
            "no --linux-pathb input (the Linux x64 GitHub legs of the exit-inputs dispatch)".into(),
        );
    };
    // V15-D16 reads every same-model pair of the dispatch: a leg that was not read, or that has
    // no CPU model, makes that set unknown, so no verdict and no fallback (EI2 Codex r1 F1).
    let unread: Vec<String> = unread_legs(wj, "win32-x64")
        .into_iter()
        .chain(unread_legs(lj, "linux-x64"))
        .collect();
    if !unread.is_empty() {
        return Ok((
            HostForm::NoAgreement,
            format!(
                "Windows / Linux x64 same-CPU-model form (V15-D16; {} and {}): a leg of the dispatch was not read ({}), so the same-model pairs are not known; run the dispatch again",
                wp.display(),
                lp.display(),
                unread.join("; ")
            ),
        ));
    }
    let (win, lin) = (compare_legs(wj, "win32-x64"), compare_legs(lj, "linux-x64"));
    // A leg that the reader did not give (no runners row) or gave twice is in neither check
    // above: each compare must hold each matrix leg once (EI2 Codex r2 F1).
    let short: Vec<String> = [(&win, MATRIX_LEGS[0]), (&lin, MATRIX_LEGS[1])]
        .into_iter()
        .filter_map(|(legs, (p, n))| {
            let names: BTreeSet<&str> = legs.iter().map(|l| leg_name(l)).collect();
            (legs.len() != n || names.len() != n).then(|| {
                format!(
                    "{p}: {} legs, {} names, {n} needed",
                    legs.len(),
                    names.len()
                )
            })
        })
        .collect();
    if !short.is_empty() {
        return Ok((
            HostForm::NoAgreement,
            format!(
                "Windows / Linux x64 same-CPU-model form (V15-D16; {} and {}): the compares do not hold each pathb leg of the matrix once ({}), so the same-model pairs are not known; run the dispatch again",
                wp.display(),
                lp.display(),
                short.join("; ")
            ),
        ));
    }
    // EI2-R1, EI2-R3-LEG-SLOT: legs of 2 runs, or a slot that is missing or doubled, make the
    // same-model pairs of one dispatch not known: no verdict and no fallback.
    let prov = match provenance(&[("win32-x64", &win), ("linux-x64", &lin)]) {
        Ok(t) => t,
        Err(e) => {
            return Ok((
                HostForm::NoAgreement,
                format!(
                    "Windows / Linux x64 same-CPU-model form (V15-D16; {} and {}): the legs are not of one exit-inputs dispatch with each pathb slot once ({}), so the same-model pairs are not known; read both compares of one dispatch",
                    wp.display(),
                    lp.display(),
                    e.join("; ")
                ),
            ));
        }
    };
    let pairs: Vec<(&Json, &Json)> = win
        .iter()
        .flat_map(|w| {
            lin.iter()
                .filter(|l| leg_cpu(l) == leg_cpu(w))
                .map(|l| (*w, *l))
        })
        .collect();
    if pairs.is_empty() {
        return Err(format!(
            "{prov}; no same-CPU-model pair of a Windows and a Linux x64 GitHub run (Windows: {}; Linux x64: {})",
            models(&win),
            models(&lin)
        ));
    }
    let mut why = Vec::new();
    let mut commits = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for leg in pairs.iter().flat_map(|(w, l)| [*w, *l]) {
        let key = (leg.get("platform").and_then(Json::str), leg_name(leg));
        if seen.insert(key) {
            why.extend(leg_why(c, leg, builds, &mut commits));
        }
    }
    if commits.len() > 1 {
        why.push(format!("the legs name {} commits, not one", commits.len()));
    }
    let valid = why.is_empty();
    let mut parts = Vec::new();
    let (mut all_above, mut any_below) = (true, false);
    for &b in builds {
        for &id in ids {
            let Some(&(_, item, _)) = PATHB_ITEMS.iter().find(|(i, _, _)| *i == id) else {
                continue;
            };
            let mut ivs = Vec::new();
            let mut texts = Vec::new();
            let mut whole = true;
            for (w, l) in &pairs {
                let pair = format!("{}/{}", leg_name(w), leg_name(l));
                match (leg_interval(w, b, item), leg_interval(l, b, item)) {
                    (Ok(wi), Ok(li)) => {
                        let (lo, hi) = (wi.0 / li.1, wi.1 / li.0);
                        texts.push(format!("{pair} [{lo:.3}, {hi:.3}]"));
                        ivs.push((wi, li));
                    }
                    (wi, li) => {
                        whole = false;
                        let e = |x: Result<(f64, f64), String>| x.err().unwrap_or_default();
                        texts.push(format!(
                            "{pair}: {}",
                            [e(wi), e(li)]
                                .into_iter()
                                .filter(|x| !x.is_empty())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                }
            }
            let side = if whole { cross_side(&ivs) } else { None };
            all_above &= side == Some(true);
            any_below |= side == Some(false);
            let word = match side {
                Some(true) => "AGREE above",
                Some(false) => "AGREE below",
                None => "NO AGREEMENT",
            };
            parts.push(format!("{b} item {item}: {}: {word}", texts.join(", ")));
        }
    }
    let form = if !valid {
        HostForm::NoAgreement
    } else if any_below {
        HostForm::AgreeBelow
    } else if all_above {
        HostForm::AgreeAbove
    } else {
        HostForm::NoAgreement
    };
    let names = pairs
        .iter()
        .map(|(w, l)| {
            format!(
                "{}/{} ({})",
                leg_name(w),
                leg_name(l),
                leg_cpu(w).unwrap_or("?")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    Ok((
        form,
        format!(
            "Windows / Linux x64 on same-CPU-model GitHub runner pairs (V15-D16: the same hardware class of 10-quality.md:84; the interval >= {WINDOWS_MIN} needed, 09:66; not a gate host, 10:157; {} and {}): pairs {names}; {}{}; {prov}",
            wp.display(),
            lp.display(),
            parts.join("; "),
            if valid {
                String::new()
            } else {
                format!("; not a valid reading of this tree: {}", why.join("; "))
            }
        ),
    ))
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
            "third_party/icu4x.commit",
            "third_party/patches/icu4x/0001.patch",
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

    /// M2EXIT-ICU4X-FRESH: a change of the icu4x fork pin or of its patch set alone (no change in
    /// `crates`) makes the same cells OPEN as a change in `crates` does
    /// (`stale_dirty_or_wrong_host_inputs`): ep9_temporal and ep9_intl build icu_calendar from
    /// that checkout.
    #[test]
    fn an_icu4x_pin_or_patch_change_makes_inputs_stale() {
        for (k, path) in [
            "third_party/icu4x.commit",
            "third_party/patches/icu4x/0001.patch",
        ]
        .iter()
        .enumerate()
        {
            let r = repo(&format!("icu4x{k}"));
            let o = all(&r, "gate-pass.json");
            let base = run(&o).unwrap();
            write(&r.root, path, "y\n");
            git_in(&r.root, &["commit", "-q", "-am", "icu4x"]);
            let rep = run(&o).unwrap();
            for i in [0, 1, 2, 3, 4, 5, 6, 9, 10] {
                assert_ne!(
                    base.cells[i].evidence, rep.cells[i].evidence,
                    "{path}: {i}: the evidence did not change"
                );
                assert_eq!(
                    rep.cells[i].status,
                    Status::Open,
                    "{path}: {i}: {}",
                    rep.cells[i].evidence
                );
                assert!(
                    rep.cells[i].evidence.contains("changed between"),
                    "{path}: {i}: {}",
                    rep.cells[i].evidence
                );
            }
        }
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

    /// P0-WININT (M6-ORCH-139): the Windows cells read the ratio interval, not the medians. A
    /// Windows jit Octane reading with median 1.28 and interval [1.15, 1.40] against the Linux
    /// x64 1.33 [1.32202, 1.33798]: the median ratio 0.962 read PASS; the interval [0.860,
    /// 1.059] holds 0.9, so OPEN (marginal). [1.0, 1.1] (median 1.05) is below: FAIL.
    #[test]
    fn windows_reads_the_cross_host_ratio_interval() {
        let r = repo("windows-interval");
        let base = fixture("gate-win.json").replace(SHA, &r.head);
        let jit_oct = "\"value\": 1.4,\n   \"interval\": [\n    1.3916,\n    1.4084\n   ]";
        assert!(base.contains(jit_oct));
        let at = base.find("\"id\": \"octane7-vs-qjs\"").unwrap();
        let with = |v: &str| {
            let mut t = base.clone();
            let k = at + t[at..].find(jit_oct).unwrap();
            t.replace_range(k..k + jit_oct.len(), v);
            t
        };
        let cases = [
            (
                "\"value\": 1.28, \"interval\": [1.15, 1.4]",
                Status::Open,
                "jit octane7-vs-qjs Windows / Linux x64 = 0.962 [0.860, 1.059] marginal",
            ),
            (
                "\"value\": 1.05, \"interval\": [1.0, 1.1]",
                Status::Fail,
                "jit octane7-vs-qjs Windows / Linux x64 = 0.789 [0.747, 0.832] below",
            ),
        ];
        for (n, (v, want, text)) in cases.into_iter().enumerate() {
            let f = format!("inputs/gate-win-{n}.json");
            write(&r.root, &f, &with(v));
            let o = Options {
                gate: Some(r.root.join(&f)),
                linux_gate: Some(input(&r, "gate-win-linux.json")),
                ..opts(&r, WINDOWS_X64)
            };
            let c = &run(&o).unwrap().cells[0];
            assert_eq!(c.status, want, "{}", c.evidence);
            assert!(c.evidence.contains(text), "{text}: {}", c.evidence);
        }
        // The fixture pair itself: each interval is above 0.9 (the median 0.950 one too).
        let o = Options {
            gate: Some(input(&r, "gate-win.json")),
            linux_gate: Some(input(&r, "gate-win-linux.json")),
            ..opts(&r, WINDOWS_X64)
        };
        let c = &run(&o).unwrap().cells[0];
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        assert!(
            c.evidence
                .contains("jitless octane7-vs-qjs Windows / Linux x64 = 0.950 [0.939, 0.961]"),
            "{}",
            c.evidence
        );
        let ts = speed_interval(TS, &parse_json("{\"interval\": [2.0, 4.0]}").unwrap());
        assert_eq!(ts, Some((0.25, 0.5)));
        assert_eq!(
            speed_interval("x", &parse_json("{\"interval\": [2.0, 1.0]}").unwrap()),
            None
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

    /// FORK-CHECKOUT-ATTEST: item 5 reads the fork line of the runner's build. Every fork
    /// pinned is fresh; a stale or absent fork, a missing field or a line in another form is
    /// OPEN (the old reader read all of them as fresh).
    #[test]
    fn item5_reads_the_runner_fork_line() {
        let r = repo("t262-forks");
        let full = fixture("exec.json").replace(SHA, &r.head);
        let pinned = "\"runner_forks\": \"regress=pinned icu4x=pinned\"";
        assert!(full.contains(pinned), "the fixture names the fork line");
        let cases = [
            (pinned.to_string(), None),
            (
                "\"runner_forks\": \"regress=pinned icu4x=stale\"".to_string(),
                Some(
                    "stale input: the icu4x fork checkout was not its pin plus its patches (runner_forks: regress=pinned icu4x=stale)",
                ),
            ),
            (
                "\"runner_forks\": \"regress=absent icu4x=pinned\"".to_string(),
                Some("the regress fork checkout was not its pin plus its patches"),
            ),
            (
                "\"runner_forks\": \"regress=pinned\"".to_string(),
                Some("its fork line \"regress=pinned\" is in another form (runner_forks)"),
            ),
            (
                "\"runner_forks_none\": 0".to_string(),
                Some("it names no fork checkout state (runner_forks;"),
            ),
        ];
        for (k, (field, want)) in cases.iter().enumerate() {
            write(
                &r.root,
                &format!("inputs/exec-f{k}.json"),
                &full.replace(pinned, field),
            );
            let o = Options {
                exec: Some(r.root.join(format!("inputs/exec-f{k}.json"))),
                ..all(&r, "gate-pass.json")
            };
            let c = &run(&o).unwrap().cells[4];
            match want {
                None => assert!(c.evidence.contains("test262 PASS"), "{}", c.evidence),
                Some(w) => {
                    assert_eq!(c.status, Status::Open, "{field}: {}", c.evidence);
                    assert!(c.evidence.contains("test262 OPEN"), "{}", c.evidence);
                    assert!(c.evidence.contains(w), "{w}: {}", c.evidence);
                }
            }
        }
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
        // A fast path with 0 cases is OPEN; a mismatch is FAIL. A Switch row (P0-FASTHITS):
        // 0 hits on or a hit off is FAIL, no hit count is OPEN.
        for (from, to, want) in [
            ("\"cases\": 4", "\"cases\": 0", Status::Open),
            ("\"hits_on\": 3", "\"hits_on\": 0", Status::Fail),
            ("\"hits_off\": 0", "\"hits_off\": 1", Status::Fail),
            (", \"hits_on\": 3, \"hits_off\": 0", "", Status::Open),
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

    /// P0-BUILDID (M7-P5 Codex fixer F2, Claude reviewer F1): the paths whose changes make
    /// `ep9 --version` say "dirty" (`crates/ep9_cli/build.rs` ENGINE_PATHS) are
    /// `tools/bench/cross/kernels.mjs` TREE_PATHS without `tools/bench`, and this file's
    /// [`ENGINE_PATHS`]. Reads the live sources (the m1-exit `T262_FRESH_PATHS` test form).
    #[test]
    fn the_build_line_engine_paths_are_the_bench_tree_paths() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let quoted = |s: &str| -> Vec<String> {
            s.split('"')
                .skip(1)
                .step_by(2)
                .map(str::to_string)
                .collect()
        };
        let build = std::fs::read_to_string(root.join("crates/ep9_cli/build.rs")).unwrap();
        let start = build.find("const ENGINE_PATHS").expect("ENGINE_PATHS");
        let body = &build[start..start + build[start..].find("];").unwrap()];
        let built = quoted(&body[body.find('=').unwrap()..]);
        let kernels = std::fs::read_to_string(root.join("tools/bench/cross/kernels.mjs")).unwrap();
        let line = |name: &str| -> &str {
            kernels
                .lines()
                .find(|l| l.starts_with(&format!("export const {name} = [")))
                .unwrap_or_else(|| panic!("kernels.mjs: {name}"))
        };
        let tree: Vec<String> = line("TREE_PATHS")
            .split("...")
            .skip(1)
            .map(|n| n.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_'))
            .flat_map(|n| quoted(line(n)))
            .filter(|p| p != "tools/bench")
            .collect();
        assert_eq!(tree.len(), 8, "{tree:?}");
        assert_eq!(built, tree, "crates/ep9_cli/build.rs ENGINE_PATHS");
        assert_eq!(built, ENGINE_PATHS, "m2_exit.rs ENGINE_PATHS");
    }

    /// P0-BUILDID: a mem or JetStream run whose ep9 binary prints no build line, or was built
    /// dirty, names no ep9 commit (the tools write null), so the cell is OPEN and says why.
    #[test]
    fn a_binary_with_no_clean_build_line_is_open() {
        let r = repo("buildid");
        let head = format!("\"ep9\": \"{}\", \"runner_dirty\"", r.head);
        let mem = fixture("mem.json")
            .replace(SHA, &r.head)
            .replace(&head, "\"ep9\": null, \"binary\": null, \"runner_dirty\"");
        assert!(mem.contains("\"binary\": null"));
        write(&r.root, "inputs/mem-nobuild.json", &mem);
        let o = Options {
            mem: Some(r.root.join("inputs/mem-nobuild.json")),
            ..opts(&r, HOST)
        };
        let c = &run(&o).unwrap().cells[3];
        assert_eq!(c.status, Status::Open);
        assert!(
            c.evidence.contains(
                "it names no ep9 commit (an ep9 binary prints no build line (`ep9 --version`, \
                 P0-BUILDID))"
            ),
            "{}",
            c.evidence
        );
        let js = fixture("jetstream.json").replace(SHA, &r.head).replace(
            &format!("\"ep9\": {{ \"commit\": \"{}\", \"dirty\": false }}", r.head),
            &format!(
                "\"ep9\": {{ \"commit\": null, \"dirty\": false, \"binary\": [{{ \"commit\": \"{}\", \"dirty\": true }}] }}",
                r.head
            ),
        );
        assert!(js.contains("\"binary\""));
        write(&r.root, "inputs/js-dirty.json", &js);
        let o = Options {
            jetstream: Some(r.root.join("inputs/js-dirty.json")),
            ..opts(&r, HOST)
        };
        let c = &run(&o).unwrap().cells[6];
        assert_eq!(c.status, Status::Open);
        assert!(
            c.evidence
                .contains("an ep9 binary was built with uncommitted engine changes"),
            "{}",
            c.evidence
        );
        assert_eq!(
            with_binary_why("x".into(), None),
            "x",
            "a JSON from before P0-BUILDID keeps the plain reason"
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

    // ---- the host-cell rule (V13-D37) -------------------------------------------------------

    /// The speed cells: items 1, 2, 3, 6, 10 and D11.
    const SPEED: [usize; 6] = [0, 1, 2, 5, 9, 10];

    fn linux_pathb(r: &Repo, pathb: &str) -> Options {
        Options {
            gate: Some(input(r, "gate-linux.json")),
            pathb: Some(input(r, pathb)),
            ..opts(r, LINUX_X64)
        }
    }

    /// The 3 cell forms on Linux x64 with no gate host, one fixture each (month-7 Path B: TS
    /// AGREE below; all above; one box straddling), and the cells with no --pathb as before.
    #[test]
    fn linux_x64_with_no_gate_host_reads_the_path_b_forms() {
        let r = repo("pathb-linux");
        let above = HostForm::AgreeAbove.label();
        let below = HostForm::AgreeBelow.label();
        let none = HostForm::NoAgreement.label();
        let cases = [
            (
                "pathb-below.json",
                [above, above, below, above, below, below],
                "jit item 3 (threshold 1): box 1 0.766 [0.740-0.791] below, box 2 0.756 [0.742-0.765] below: AGREE below",
            ),
            (
                "pathb-above.json",
                [above; 6],
                "jitless item 3 (threshold 1): box 1 1.031 [1.012-1.052] above, box 2 1.027 [1.008-1.049] above: AGREE above",
            ),
            (
                "pathb-none.json",
                [above, none, above, above, none, above],
                "jit item 2 (threshold 1): box 1 1.064 [1.054-1.094] above, box 2 1.004 [0.991-1.018] straddles: NO AGREEMENT",
            ),
        ];
        for (f, want, text) in cases {
            let rep = run(&linux_pathb(&r, f)).unwrap();
            for (i, w) in SPEED.iter().zip(want) {
                let c = &rep.cells[*i];
                assert_eq!(c.status, Status::Open, "{f} {i}: {}", c.evidence);
                assert_eq!(rep.forms[*i].map(HostForm::label), Some(w), "{f} {i}");
                assert!(
                    c.evidence.starts_with(&format!("{w}: Path B ")),
                    "{}",
                    c.evidence
                );
                assert!(
                    rep.table
                        .contains(&format!("| {} | {} | {w} |", LABELS[*i], TITLES[*i])),
                    "{}",
                    rep.table
                );
                assert!(
                    rep.markdown
                        .contains(&format!("- {LINUX_X64} {w}: Path B "))
                );
                assert!(!rep.markdown.contains(&format!("{w}: {w}")));
            }
            let all: String = rep.cells.iter().map(|c| c.evidence.clone()).collect();
            assert!(all.contains(text), "{f}: {text}");
            // The --gate reading stays in the evidence.
            assert!(
                rep.cells[0]
                    .evidence
                    .contains("; the --gate reading (OPEN): marginal (no gate host")
            );
            assert!(!rep.missing.iter().any(|m| m.starts_with("--pathb")));
            // The other items are NOT-RUN here: the M2 exit does not hold.
            assert!(!exit_holds(&rep.cells, &rep.forms));
        }
        let rep = run(&linux_pathb(&r, "pathb-below.json")).unwrap();
        assert!(
            rep.markdown.contains("- Result: not every cell is PASS; items 3, 10, D11 read FAIL-SIDE (no gate host): AGREE below: a Path B 2-box agreement below the threshold blocks the M2 exit (V13-D37)."),
            "{}",
            rep.markdown
        );
        // No --pathb: the cells as before, and a missing-input line.
        let o = Options {
            gate: Some(input(&r, "gate-linux.json")),
            ..opts(&r, LINUX_X64)
        };
        let rep = run(&o).unwrap();
        for i in SPEED {
            assert_eq!(rep.forms[i], None);
            assert!(rep.cells[i].evidence.starts_with("marginal (no gate host"));
        }
        assert!(rep.missing.iter().any(|m| {
            m.starts_with("--pathb (items 1, 2, 3, 6, 10, D11 with no gate host (V13-D37))")
        }));
    }

    /// A gate-host reading decides as before (Path B is not read); a mac or Linux a64 column
    /// takes no --pathb; with no --gate, Path B alone gives the form.
    #[test]
    fn a_gate_host_reading_decides_and_pathb_is_for_the_x64_hosts() {
        let r = repo("pathb-gate");
        let lin = fixture("gate-linux.json")
            .replace(SHA, &r.head)
            .replace("\"gate_host\": false", "\"gate_host\": true");
        write(&r.root, "inputs/gate-host.json", &lin);
        let o = Options {
            gate: Some(r.root.join("inputs/gate-host.json")),
            pathb: Some(input(&r, "pathb-below.json")),
            ..opts(&r, LINUX_X64)
        };
        let rep = run(&o).unwrap();
        for i in SPEED {
            assert_eq!(rep.forms[i], None);
            assert!(
                !rep.cells[i].evidence.contains("Path B"),
                "{}",
                rep.cells[i].evidence
            );
            assert!(!rep.cells[i].evidence.starts_with("marginal"));
        }
        assert!(!rep.missing.iter().any(|m| m.starts_with("--pathb")));
        // Windows: the gate-host pair on one CPU model decides as before.
        let o = Options {
            gate: Some(input(&r, "gate-win.json")),
            linux_gate: Some(input(&r, "gate-win-linux.json")),
            pathb: Some(input(&r, "pathb-below.json")),
            ..opts(&r, WINDOWS_X64)
        };
        let rep = run(&o).unwrap();
        assert_eq!(rep.forms, vec![None; 11]);
        assert_eq!(
            rep.cells[0].status,
            Status::Pass,
            "{}",
            rep.cells[0].evidence
        );
        // Path B alone (no --gate): the form, with no --gate text.
        let o = Options {
            pathb: Some(input(&r, "pathb-above.json")),
            ..opts(&r, LINUX_X64)
        };
        let rep = run(&o).unwrap();
        assert_eq!(rep.forms[2], Some(HostForm::AgreeAbove));
        assert!(!rep.cells[2].evidence.contains("--gate reading"));
        assert!(rep.missing.iter().any(|m| m.starts_with("--gate")));
        // --pathb on the mac or on Linux a64 is a usage error: no mac cell changes.
        for host in [HOST, LINUX_A64] {
            let e = parse_args(
                &[
                    "--host".into(),
                    host.into(),
                    "--out".into(),
                    "x".into(),
                    "--pathb".into(),
                    "y".into(),
                ],
                PathBuf::new(),
            )
            .unwrap_err();
            assert!(e.contains("--pathb is for --host"), "{e}");
        }
        assert!(
            run(&Options {
                pathb: Some(input(&r, "gate-linux.json")),
                ..opts(&r, LINUX_X64)
            })
            .unwrap_err()
            .contains("not an ep9-pathb-compare/v1 file")
        );
    }

    /// Windows x64 with no gate host reads a Windows x64 Path B only: a Linux x64 compare gives
    /// NO AGREEMENT.
    #[test]
    fn windows_reads_a_windows_path_b() {
        let r = repo("pathb-win");
        let win = fixture("gate-linux.json")
            .replace(SHA, &r.head)
            .replace("\"platform\": \"linux-x64\"", "\"platform\": \"win32-x64\"");
        write(&r.root, "inputs/gate-win-box.json", &win);
        let pb = fixture("pathb-above.json")
            .replace(SHA, &r.head)
            .replace("\"platform\": \"linux-x64\"", "\"platform\": \"win32-x64\"");
        write(&r.root, "inputs/pathb-win.json", &pb);
        let o = |pathb: PathBuf| Options {
            gate: Some(r.root.join("inputs/gate-win-box.json")),
            linux_gate: Some(input(&r, "gate-linux.json")),
            pathb: Some(pathb),
            ..opts(&r, WINDOWS_X64)
        };
        let rep = run(&o(input(&r, "pathb-above.json"))).unwrap();
        for i in SPEED {
            assert_eq!(rep.forms[i], Some(HostForm::NoAgreement));
            assert!(
                rep.cells[i]
                    .evidence
                    .contains("the compare names the platform linux-x64, not win32-x64"),
                "{}",
                rep.cells[i].evidence
            );
        }
        let rep = run(&o(r.root.join("inputs/pathb-win.json"))).unwrap();
        for i in SPEED {
            assert_eq!(
                rep.forms[i],
                Some(HostForm::AgreeAbove),
                "{}",
                rep.cells[i].evidence
            );
            assert!(
                rep.cells[i]
                    .evidence
                    .contains("; the --gate reading (OPEN): marginal (no gate host: a dual-boot")
            );
        }
    }

    /// A compare that is not a valid reading of this tree gives NO AGREEMENT, even when its
    /// boxes agree below (a stale reading is not this tree's evidence). A run check is per
    /// build: a bad jit run check leaves the D11 cell (the jitless build only) as it was.
    #[test]
    fn an_invalid_path_b_reads_no_agreement() {
        let r = repo("pathb-invalid");
        let base = fixture("pathb-below.json").replace(SHA, &r.head);
        let first_rounds = "\"rounds\": 31";
        let all: &[usize] = &SPEED;
        let jit: &[usize] = &[0, 1, 2, 5, 9];
        let cases: [(&str, String, &str, &[usize]); 6] = [
            (
                "stale",
                fixture("pathb-below.json"),
                "the compare is stale: ",
                all,
            ),
            (
                "rounds",
                base.replacen(first_rounds, "\"rounds\": 15", 1),
                "box 1 jit: fewer than 31 paired rounds",
                jit,
            ),
            (
                "commit_ok",
                base.replacen("\"commit_ok\": true", "\"commit_ok\": false", 1),
                "box 1 jit: the run is not of the compare commit, or its binary is dirty",
                jit,
            ),
            (
                "guard",
                base.replacen("\"pass\": true", "\"pass\": false", 1),
                "box 1 jit: the load guard before the run did not pass",
                jit,
            ),
            (
                "platform",
                base.replace("\"platform\": \"linux-x64\",\n", ""),
                "the compare names the platform none, not linux-x64",
                all,
            ),
            (
                "box",
                base.replace("\"box\": 2,", "\"box\": 3,"),
                "no run check of box 2 build jit",
                all,
            ),
        ];
        for (n, text, why, cells) in cases {
            let f = format!("inputs/pathb-{n}.json");
            write(&r.root, &f, &text);
            let o = Options {
                pathb: Some(r.root.join(&f)),
                ..opts(&r, LINUX_X64)
            };
            let rep = run(&o).unwrap();
            for &i in cells {
                assert_eq!(rep.forms[i], Some(HostForm::NoAgreement), "{n} {i}");
            }
            if cells.len() == 5 {
                assert_eq!(rep.forms[10], Some(HostForm::AgreeBelow), "{n}");
            }
            assert!(
                rep.cells[2].evidence.contains(why),
                "{n}: {}",
                rep.cells[2].evidence
            );
            assert!(
                rep.cells[2]
                    .evidence
                    .contains("not a valid reading of this tree")
            );
        }
        // A cell whose threshold is not the M2 one, a missing cell, and a result field that
        // says something else: that part does not agree.
        let parts = [
            (
                base.replacen("\"threshold\": 1.33", "\"threshold\": 1.2", 1),
                0,
                "jit item 1: the compare threshold is not the M2 threshold 1.33",
            ),
            (
                base.replacen("\"cell\": \"item 6:", "\"cell\": \"item six:", 1),
                5,
                "jit item 6: 0 cells (one needed)",
            ),
            (
                base.replacen(
                    "\"result\": \"AGREE above\"",
                    "\"result\": \"NO AGREEMENT\"",
                    1,
                ),
                0,
                "AGREE above (the compare says NO AGREEMENT)",
            ),
        ];
        for (k, (text, i, why)) in parts.into_iter().enumerate() {
            let f = format!("inputs/pathb-part-{k}.json");
            write(&r.root, &f, &text);
            let o = Options {
                pathb: Some(r.root.join(&f)),
                ..opts(&r, LINUX_X64)
            };
            let rep = run(&o).unwrap();
            assert_eq!(rep.forms[i], Some(HostForm::NoAgreement), "{k}");
            assert!(
                rep.cells[i].evidence.contains(why),
                "{k}: {}",
                rep.cells[i].evidence
            );
        }
    }

    /// The verdict line and the exit code follow V13-D37: PASS or N/A everywhere, or the cells
    /// that are not read MARGINAL (no gate host): AGREE above.
    #[test]
    fn the_verdict_line_follows_v13_d37() {
        let r = repo("pathb-verdict");
        let o = opts(&r, LINUX_X64);
        let c = Ctx {
            o: &o,
            gate: None,
            linux_gate: None,
            pathb: None,
            linux_pathb: None,
            mem: None,
            exec: None,
            fastdiff: None,
            jetstream: None,
            fuzz: Vec::new(),
            p_gates: None,
        };
        let pass = || (cell(Status::Pass, "x"), None);
        let form = |f: HostForm| (cell(Status::Open, f.label()), Some(f));
        let with = |at: &[(usize, HostForm)]| {
            let mut v: Vec<_> = (0..11).map(|_| pass()).collect();
            for &(i, f) in at {
                v[i] = form(f);
            }
            render(&c, v, Vec::new(), Vec::new())
        };
        let rep = with(&[]);
        assert!(exit_holds(&rep.cells, &rep.forms));
        assert!(rep.markdown.contains(
            "- Result: every item PASS (N/A cells are by design): the M2 exit holds on this host."
        ));
        let rep = with(&[(2, HostForm::AgreeAbove), (9, HostForm::AgreeAbove)]);
        assert!(exit_holds(&rep.cells, &rep.forms));
        assert!(
            rep.markdown.contains("- Result: the M2 exit holds on this host by the host-cell rule (V13-D37): every other item PASS or N/A, and items 3, 10 read MARGINAL (no gate host): AGREE above (a Path B 2-box agreement on the pass side; each is read again on the gate host when one exists, and a fail there reopens M2)."),
            "{}",
            rep.markdown
        );
        assert!(
            rep.table
                .contains("| 3 | TS workload <= 2.8 s | MARGINAL (no gate host): AGREE above |")
        );
        let rep = with(&[(2, HostForm::AgreeBelow), (0, HostForm::AgreeAbove)]);
        assert!(!exit_holds(&rep.cells, &rep.forms));
        assert!(
            rep.markdown
                .contains("; item 3 reads FAIL-SIDE (no gate host): AGREE below: ")
        );
        let rep = with(&[(1, HostForm::NoAgreement)]);
        assert!(!exit_holds(&rep.cells, &rep.forms));
        assert!(rep.markdown.contains("- Result: not every cell is PASS."));
        // A cell that is not a host cell and not PASS blocks the exit with an AGREE above.
        let mut v: Vec<_> = (0..11).map(|_| pass()).collect();
        v[2] = form(HostForm::AgreeAbove);
        v[7] = (cell(Status::Open, "item 8"), None);
        let rep = render(&c, v, Vec::new(), Vec::new());
        assert!(!exit_holds(&rep.cells, &rep.forms));
    }

    /// M9-P1 EXIT-INPUTS (plan v14 §4.1 (1), (6)): the windows Path B compare that
    /// `.github/scripts/exit-inputs.mjs pathb` writes (fixture pathb-win.json, made by its
    /// test) is a valid win32-x64 reading. With no --linux-pathb, its `legs` block gives NO
    /// AGREEMENT (option B); with the block cut (pathb-run-nolegs.json, the older form), item 1
    /// has the QuickJS runs of V15-D17 (AGREE above); items 2 and 6 read AGREE above; the
    /// made-up TS cells read AGREE below, so items 3, 10 and D11 read FAIL-SIDE. On Linux x64
    /// the same compare is NO AGREEMENT.
    #[test]
    fn windows_reads_the_exit_inputs_path_b() {
        use HostForm::{AgreeAbove, AgreeBelow, NoAgreement};
        let r = repo("pathb-exit-inputs");
        let o = Options {
            pathb: Some(input(&r, "pathb-win.json")),
            ..opts(&r, WINDOWS_X64)
        };
        let rep = run(&o).unwrap();
        for i in SPEED {
            assert_eq!(rep.forms[i], Some(NoAgreement), "{}", rep.cells[i].evidence);
        }
        let o = Options {
            pathb: Some(input(&r, "pathb-run-nolegs.json")),
            ..opts(&r, WINDOWS_X64)
        };
        let rep = run(&o).unwrap();
        let forms: Vec<Option<HostForm>> = SPEED.iter().map(|&i| rep.forms[i]).collect();
        assert_eq!(
            forms,
            [
                Some(AgreeAbove),
                Some(AgreeAbove),
                Some(AgreeBelow),
                Some(AgreeAbove),
                Some(AgreeBelow),
                Some(AgreeBelow)
            ],
            "{}",
            rep.table
        );
        for i in SPEED {
            assert!(
                !rep.cells[i].evidence.contains("not a valid reading"),
                "{}",
                rep.cells[i].evidence
            );
        }
        assert!(
            rep.cells[0].evidence.contains(
                "jit item 1 (threshold 1.33): box 1 1.400 [1.370-1.430] above, box 2 1.410 [1.380-1.440] above: AGREE above"
            ),
            "{}",
            rep.cells[0].evidence
        );
        assert!(
            rep.cells[2]
                .evidence
                .contains("jit item 3 (threshold 1): box 1 0.950 [0.930-0.970] below, box 2 0.960 [0.940-0.980] below: AGREE below"),
            "{}",
            rep.cells[2].evidence
        );
        let o = Options {
            pathb: Some(input(&r, "pathb-win.json")),
            ..opts(&r, LINUX_X64)
        };
        let rep = run(&o).unwrap();
        for i in SPEED {
            assert_eq!(rep.forms[i], Some(NoAgreement));
            assert!(
                rep.cells[i]
                    .evidence
                    .contains("the compare names the platform win32-x64, not linux-x64")
            );
        }
    }

    /// M9-P1 EXIT-INPUTS (plan v14 §4.1 (4), (5)): a Windows MEMREAD run whose load guard reads
    /// the typeperf source passes item 4; a win32 run whose load source is loadavg (always 0 on
    /// Windows) has no load guard: mem.mjs writes the guard as failed, so the cell is OPEN.
    #[test]
    fn item4_windows_typeperf_guard_and_a_loadavg_run_has_no_load_guard() {
        let r = repo("mem-win");
        let o = |f: PathBuf| Options {
            mem: Some(f),
            ..opts(&r, WINDOWS_X64)
        };
        let c = &run(&o(input(&r, "mem-win.json"))).unwrap().cells[3];
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        let none = "{ \"load1\": 0, \"limit\": 0.5, \"pass\": false, \"why\": \"no load guard: the load source loadavg is always 0 on Windows\", \"source\": \"loadavg\" }";
        let old = fixture("mem-win.json")
            .replace(SHA, &r.head)
            .replace("\"load_source\": \"typeperf\"", "\"load_source\": \"loadavg\"")
            .replace(
                "{ \"load1\": 0.41, \"limit\": 0.92, \"pass\": true, \"why\": null, \"source\": \"typeperf\" }",
                none,
            )
            .replace(
                "{ \"load1\": 0.38, \"limit\": 0.92, \"pass\": true, \"why\": null, \"source\": \"typeperf\" }",
                none,
            );
        assert_eq!(old.matches("no load guard").count(), 2);
        write(&r.root, "inputs/mem-win-loadavg.json", &old);
        let c = &run(&o(r.root.join("inputs/mem-win-loadavg.json")))
            .unwrap()
            .cells[3];
        assert_eq!(c.status, Status::Open);
        assert!(
            c.evidence
                .contains("the load guard did not pass before and after the run"),
            "{}",
            c.evidence
        );
        // The Windows run on another host column: the platform check (X12).
        let rep = run(&Options {
            mem: Some(input(&r, "mem-win.json")),
            ..opts(&r, LINUX_X64)
        })
        .unwrap();
        assert_eq!(
            rep.cells[3].status,
            Status::Fail,
            "{}",
            rep.cells[3].evidence
        );
    }

    /// M9-P1 EXIT-INPUTS (plan v14 §4.1 (3)): the exit-form P-gate file (the first table with
    /// Gate and Result columns, docs/measurements/2026-11-16-p-gates-exit-form.md) reads PASS;
    /// a table with a `Trips now?` column and no Result column (the 2026-11-09 form) gives no
    /// row, so the cell is OPEN.
    #[test]
    fn item9_reads_the_exit_form_p_gate_file() {
        let rows = p_gate_rows(&fixture("p-gates-exit-form.md"));
        let want: Vec<(String, String)> = ["P1", "P2", "P4"]
            .iter()
            .map(|g| (g.to_string(), "not tripped".to_string()))
            .collect();
        assert_eq!(rows, want);
        let r = repo("p-gates-exit");
        let exit = "docs/measurements/2026-11-16-p-gates-exit-form.md";
        let report = "docs/measurements/2026-11-09-p-gates-report.md";
        write(&r.root, exit, &fixture("p-gates-exit-form.md"));
        write(
            &r.root,
            report,
            "| Gate | Trigger | Reading | Trips now? |\n|---|---|---|---|\n| P1 | t | r | No |\n| P2 | t | r | No |\n| P4 | t | r | No |\n",
        );
        git_in(&r.root, &["add", "-A"]);
        git_in(&r.root, &["commit", "-q", "-m", "p-gates"]);
        let o = |f: &str| Options {
            p_gates: Some(r.root.join(f)),
            ..opts(&r, HOST)
        };
        let c = &run(&o(exit)).unwrap().cells[8];
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        assert!(
            c.evidence
                .contains("P1 not tripped, P2 not tripped, P4 not tripped"),
            "{}",
            c.evidence
        );
        let c = &run(&o(report)).unwrap().cells[8];
        assert_eq!(c.status, Status::Open);
        assert!(c.evidence.contains("no P1 row"), "{}", c.evidence);
    }

    /// M9-P1 EXIT-INPUTS (plan v14 §4.1 (1)): the per-platform MEMREAD and JetStream inputs of
    /// the exit-inputs job. Linux arm64 has no d8 (jsvu has no linux-arm64 V8 build,
    /// third_party/shells.toml): since V15-D18 the d8 half is N/A by design there, so a run with
    /// or without d8 rows passes on the node cap. The JetStream run names its node platform and
    /// arch per host.
    #[test]
    fn per_platform_mem_and_jetstream_inputs() {
        let r = repo("per-platform");
        let arm = fixture("mem.json").replace(SHA, &r.head).replace(
            "\"platform\": \"darwin-arm64\"",
            "\"platform\": \"linux-arm64\"",
        );
        write(&r.root, "inputs/mem-arm.json", &arm);
        let no_d8 = arm
            .lines()
            .filter(|l| !l.contains("\"engine\": \"d8-jitless\""))
            .collect::<Vec<_>>()
            .join("\n");
        write(&r.root, "inputs/mem-arm-no-d8.json", &no_d8);
        let mem = |f: &str| {
            run(&Options {
                mem: Some(r.root.join(f)),
                ..opts(&r, LINUX_A64)
            })
            .unwrap()
            .cells[3]
                .clone()
        };
        let c = mem("inputs/mem-arm.json");
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        let c = mem("inputs/mem-arm-no-d8.json");
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        assert!(c.evidence.contains("the d8 --jitless half: N/A by design"));
        for (host, platform, arch) in [
            (LINUX_A64, "linux", "arm64"),
            (WINDOWS_X64, "win32", "x64"),
            (LINUX_X64, "linux", "x64"),
        ] {
            let js = fixture("jetstream.json").replace(SHA, &r.head).replace(
                "\"platform\": \"darwin\", \"arch\": \"arm64\"",
                &format!("\"platform\": \"{platform}\", \"arch\": \"{arch}\""),
            );
            let f = format!("inputs/jetstream-{platform}-{arch}.json");
            write(&r.root, &f, &js);
            let o = Options {
                jetstream: Some(r.root.join(&f)),
                ..opts(&r, host)
            };
            let c = &run(&o).unwrap().cells[6];
            assert_eq!(c.status, Status::Pass, "{host}: {}", c.evidence);
            let o = Options {
                jetstream: Some(input(&r, "jetstream.json")),
                ..opts(&r, host)
            };
            assert_eq!(run(&o).unwrap().cells[6].status, Status::Fail, "{host}");
        }
    }
    /// The win-x64 same-CPU-model fixtures (plan v15 V15-D16; made by
    /// `.github/scripts/exit-inputs.test.mjs`): the windows compare pathb-win.json and a Linux
    /// x64 GitHub compare.
    fn win_cross(r: &Repo, linux: &str) -> Options {
        Options {
            pathb: Some(input(r, "pathb-win.json")),
            linux_pathb: Some(input(r, linux)),
            ..opts(r, WINDOWS_X64)
        }
    }

    /// V15-D16: each Windows leg pairs only with the Linux x64 legs of its CPU model (w2 and w3
    /// are EPYC 7763; w1 and w4 are Intel, with no Linux leg). The Linux values are a little
    /// above the Windows ones: every cross-host low end is >= 0.9, so every speed cell reads
    /// AGREE above, while the Path B form of the same windows compare reads TS AGREE below. With
    /// Linux values about 1.3x the Windows ones, every high end is < 0.9: FAIL-SIDE.
    #[test]
    fn windows_same_cpu_model_pairs_read_the_09_66_form() {
        use HostForm::{AgreeAbove, AgreeBelow, NoAgreement};
        let r = repo("pathb-cross");
        let rep = run(&win_cross(&r, "pathb-gha-linux.json")).unwrap();
        for i in SPEED {
            assert_eq!(rep.forms[i], Some(AgreeAbove), "{}", rep.cells[i].evidence);
            assert_eq!(rep.cells[i].status, Status::Open);
        }
        let ev = &rep.cells[2].evidence;
        assert!(
            ev.starts_with("MARGINAL (no gate host): AGREE above: Windows / Linux x64 on same-CPU-model GitHub runner pairs (V15-D16"),
            "{ev}"
        );
        let m = "AMD EPYC 7763 64-Core Processor";
        assert!(
            ev.contains(&format!(
                "pairs w2/l1 ({m}), w2/l2 ({m}), w3/l1 ({m}), w3/l2 ({m});"
            )),
            "{ev}"
        );
        assert!(
            ev.contains("jit item 3: w2/l1 [0.920, 0.991], w2/l2 [0.920, 0.991], w3/l1 [0.929, 1.000], w3/l2 [0.929, 1.000]: AGREE above"),
            "{ev}"
        );
        assert!(!ev.contains("Intel"), "{ev}");
        assert!(!rep.missing.iter().any(|m| m.starts_with("--linux-pathb")));
        let rep = run(&win_cross(&r, "pathb-gha-linux-fast.json")).unwrap();
        for i in SPEED {
            assert_eq!(rep.forms[i], Some(AgreeBelow), "{}", rep.cells[i].evidence);
        }
        // A leg of a pair that is not a valid run: NO AGREEMENT, never a verdict.
        let lin = fixture("pathb-gha-linux.json").replace(SHA, &r.head);
        let busy = lin.replace("\"pass\": true", "\"pass\": false");
        write(&r.root, "inputs/lin-busy.json", &busy);
        let dirty = lin.replace("\"commit_ok\": true", "\"commit_ok\": false");
        write(&r.root, "inputs/lin-dirty.json", &dirty);
        let short = lin.replace("\"rounds\": 31", "\"rounds\": 30");
        write(&r.root, "inputs/lin-short.json", &short);
        let stale = fixture("pathb-gha-linux.json").replace(SHA, &"f".repeat(40));
        write(&r.root, "inputs/lin-stale.json", &stale);
        for (f, want) in [
            (
                "lin-busy.json",
                ": the load guard before the run did not pass",
            ),
            (
                "lin-dirty.json",
                ": the run is not of the leg commit, or its binary is dirty",
            ),
            ("lin-short.json", ": fewer than 31 paired rounds"),
            ("lin-stale.json", "leg l1 is stale"),
        ] {
            let o = Options {
                linux_pathb: Some(r.root.join("inputs").join(f)),
                ..win_cross(&r, "pathb-gha-linux.json")
            };
            let rep = run(&o).unwrap();
            for i in SPEED {
                assert_eq!(
                    rep.forms[i],
                    Some(NoAgreement),
                    "{f}: {}",
                    rep.cells[i].evidence
                );
                assert!(
                    rep.cells[i]
                        .evidence
                        .contains("not a valid reading of this tree")
                        && rep.cells[i].evidence.contains(want),
                    "{f}: {}",
                    rep.cells[i].evidence
                );
            }
        }
    }

    /// V15-D16, K19: with no same-CPU-model pair (the Linux legs are EPYC 9V74), or with no
    /// --linux-pathb and an older compare (no `legs`), the cell keeps the Path B form of the
    /// windows compare and names the CPU models. Option B (EI2-R3-LINUX-PATHB-REQUIRED): with no
    /// --linux-pathb and a compare with `legs` (an exit-inputs dispatch) the cell reads NO
    /// AGREEMENT. A --linux-pathb on another host is a usage error.
    #[test]
    fn windows_with_no_same_model_pair_keeps_the_path_b_form() {
        use HostForm::{AgreeAbove, AgreeBelow};
        let r = repo("pathb-cross-none");
        let want = [
            Some(AgreeAbove),
            Some(AgreeAbove),
            Some(AgreeBelow),
            Some(AgreeAbove),
            Some(AgreeBelow),
            Some(AgreeBelow),
        ];
        let rep = run(&win_cross(&r, "pathb-gha-linux-other.json")).unwrap();
        let forms: Vec<Option<HostForm>> = SPEED.iter().map(|&i| rep.forms[i]).collect();
        assert_eq!(forms, want, "{}", rep.table);
        for i in SPEED {
            assert!(
                rep.cells[i].evidence.contains("no same-CPU-model pair of a Windows and a Linux x64 GitHub run (Windows: AMD EPYC 7763 64-Core Processor, Intel Xeon Platinum 8370C; Linux x64: AMD EPYC 9V74 80-Core Processor), so the cell keeps the Path B form (V15-D16): Path B "),
                "{}",
                rep.cells[i].evidence
            );
        }
        let o = Options {
            pathb: Some(input(&r, "pathb-win.json")),
            ..opts(&r, WINDOWS_X64)
        };
        let rep = run(&o).unwrap();
        for i in SPEED {
            let ev = &rep.cells[i].evidence;
            assert_eq!(rep.forms[i], Some(HostForm::NoAgreement), "{ev}");
            assert!(
                ev.contains("the windows compare has a legs block (an exit-inputs dispatch); the Linux x64 compare is missing (pass --linux-pathb"),
                "{ev}"
            );
            assert!(!ev.contains("keeps the Path B form"), "{ev}");
        }
        assert!(rep.missing.iter().any(|m| m.starts_with("--linux-pathb")));
        let o = Options {
            pathb: Some(input(&r, "pathb-run-nolegs.json")),
            ..opts(&r, WINDOWS_X64)
        };
        let rep = run(&o).unwrap();
        let forms: Vec<Option<HostForm>> = SPEED.iter().map(|&i| rep.forms[i]).collect();
        assert_eq!(forms, want);
        assert!(rep.cells[0].evidence.contains("no --linux-pathb input"));
        assert!(rep.missing.iter().any(|m| m.starts_with("--linux-pathb")));
        let mut args: Vec<String> = [
            "--host",
            LINUX_X64,
            "--out",
            "x.md",
            "--linux-pathb",
            "l.json",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert!(
            parse_args(&args, r.root.clone())
                .unwrap_err()
                .contains("--linux-pathb is for --host x86_64-pc-windows-msvc only")
        );
        args[1] = WINDOWS_X64.into();
        assert!(parse_args(&args, r.root.clone()).is_ok());
    }

    /// EI2 Codex r1 F1: a leg of the dispatch that was not read makes the same-model pairs not
    /// known. One Linux leg not read (l1 alone would give AGREE above), or both (no pair at all,
    /// which would fall back to the Path B form): the cell reads NO AGREEMENT and names the leg.
    #[test]
    fn windows_cross_form_with_a_leg_not_read_reads_no_agreement() {
        let r = repo("pathb-cross-unread");
        let rep = run(&win_cross(&r, "pathb-gha-linux-unread.json")).unwrap();
        for i in SPEED {
            let ev = &rep.cells[i].evidence;
            assert_eq!(rep.forms[i], Some(HostForm::NoAgreement), "{ev}");
            assert!(
                ev.contains("a leg of the dispatch was not read (l2: no run-jit.json), so the same-model pairs are not known"),
                "{ev}"
            );
        }
        let none = format!(
            "{{\"schema\": \"{PATHB_SCHEMA}\", \"pair\": null, \"runners\": [{}], \"legs\": []}}",
            ["l1", "l2"]
                .map(|l| format!("{{\"leg\": \"{l}\", \"cpu\": null, \"platform\": null, \"error\": \"no run-jit.json\"}}"))
                .join(", ")
        );
        write(&r.root, "inputs/lin-none.json", &none);
        let o = Options {
            linux_pathb: Some(r.root.join("inputs/lin-none.json")),
            ..win_cross(&r, "pathb-gha-linux.json")
        };
        let rep = run(&o).unwrap();
        for i in SPEED {
            let ev = &rep.cells[i].evidence;
            assert_eq!(rep.forms[i], Some(HostForm::NoAgreement), "{ev}");
            assert!(
                ev.contains("not read (l1: no run-jit.json; l2: no run-jit.json)"),
                "{ev}"
            );
            assert!(!ev.contains("keeps the Path B form"), "{ev}");
        }
    }

    /// EI2 Codex r2 F1: a matrix leg that the reader did not give (no runners row, so no read
    /// error) or gave twice makes the same-model pairs not known: NO AGREEMENT, not the AGREE
    /// above of the legs that are present, and not the Path B fallback.
    #[test]
    fn windows_cross_form_with_a_leg_omitted_reads_no_agreement() {
        let r = repo("pathb-cross-omitted");
        let unread = fixture("pathb-gha-linux-unread.json").replace(SHA, &r.head);
        let l2 = ",\n  {\n   \"leg\": \"l2\",\n   \"cpu\": null,\n   \"platform\": null,\n   \"error\": \"no run-jit.json\"\n  }";
        assert!(unread.contains(l2));
        write(&r.root, "inputs/lin-omitted.json", &unread.replace(l2, ""));
        let lin = fixture("pathb-gha-linux.json").replace(SHA, &r.head);
        assert!(lin.contains("\"leg\": \"l2\""));
        let twice = lin.replace("\"leg\": \"l2\"", "\"leg\": \"l1\"");
        write(&r.root, "inputs/lin-twice.json", &twice);
        for (f, want) in [
            ("lin-omitted.json", "(linux-x64: 1 legs, 1 names, 2 needed)"),
            ("lin-twice.json", "(linux-x64: 2 legs, 1 names, 2 needed)"),
        ] {
            let o = Options {
                linux_pathb: Some(r.root.join("inputs").join(f)),
                ..win_cross(&r, "pathb-gha-linux.json")
            };
            let rep = run(&o).unwrap();
            for i in SPEED {
                let ev = &rep.cells[i].evidence;
                assert_eq!(rep.forms[i], Some(HostForm::NoAgreement), "{f}: {ev}");
                assert!(
                    ev.contains("the compares do not hold each pathb leg of the matrix once")
                        && ev.contains(want),
                    "{f}: {ev}"
                );
                assert!(!ev.contains("keeps the Path B form"), "{f}: {ev}");
            }
        }
    }

    /// EI2 Codex r1 F2: the TS speed of the cross-host form is 1 / the gross ep9 time (10:157),
    /// not the ratio to node --jitless. In pathb-gha-linux-ts.json the Linux legs have the TS
    /// ratios of the Windows legs, but 3/4 of their gross time: item 3 (and items 10, D11, which
    /// hold it) read AGREE below; items 1, 2, 6 stay AGREE above.
    #[test]
    fn windows_cross_form_reads_the_ts_speed_from_the_gross_time() {
        use HostForm::{AgreeAbove, AgreeBelow};
        let r = repo("pathb-cross-ts");
        let rep = run(&win_cross(&r, "pathb-gha-linux-ts.json")).unwrap();
        let forms: Vec<Option<HostForm>> = SPEED.iter().map(|&i| rep.forms[i]).collect();
        let want = [
            AgreeAbove, AgreeAbove, AgreeBelow, AgreeAbove, AgreeBelow, AgreeBelow,
        ];
        assert_eq!(forms, want.map(Some), "{}", rep.table);
        let ev = &rep.cells[2].evidence;
        assert!(
            ev.contains("jit item 3: w2/l1 [0.725, 0.775], w2/l2 [0.725, 0.775], w3/l1 [0.732, 0.783], w3/l2 [0.732, 0.783]: AGREE below"),
            "{ev}"
        );
    }

    /// EI2 Codex r1 F4: "unknown" is the host.mjs `cpuModel` sentinel of a failed detection. A
    /// Windows and a Linux leg that both say it are no same-model pair: the cell reads NO
    /// AGREEMENT and names each such leg ("no CPU model").
    #[test]
    fn windows_cross_form_never_pairs_the_unknown_cpu_sentinel() {
        let r = repo("pathb-cross-unknown");
        let m = "AMD EPYC 7763 64-Core Processor";
        for (f, src) in [
            ("win-unknown.json", "pathb-win.json"),
            ("lin-unknown.json", "pathb-gha-linux.json"),
        ] {
            let text = fixture(src).replace(SHA, &r.head).replace(m, "unknown");
            write(&r.root, &format!("inputs/{f}"), &text);
        }
        let o = Options {
            pathb: Some(r.root.join("inputs/win-unknown.json")),
            linux_pathb: Some(r.root.join("inputs/lin-unknown.json")),
            ..opts(&r, WINDOWS_X64)
        };
        let rep = run(&o).unwrap();
        for i in SPEED {
            let ev = &rep.cells[i].evidence;
            assert_eq!(rep.forms[i], Some(HostForm::NoAgreement), "{ev}");
            assert!(
                ev.contains("not read (w2: no CPU model; w3: no CPU model; l1: no CPU model; l2: no CPU model)"),
                "{ev}"
            );
            assert!(!ev.contains("pairs w2/l1"), "{ev}");
        }
    }

    /// The part rule of the same-CPU-model form: every low end >= 0.9 is above, every high end
    /// < 0.9 is below, a mix (or no pair) is neither.
    #[test]
    fn cross_side_reads_every_pair() {
        let p = |w: (f64, f64), l: (f64, f64)| (w, l);
        assert_eq!(cross_side(&[]), None);
        assert_eq!(cross_side(&[p((0.93, 0.97), (0.98, 1.02))]), Some(true));
        assert_eq!(cross_side(&[p((0.90, 1.00), (0.98, 1.00))]), Some(true));
        assert_eq!(cross_side(&[p((0.70, 0.80), (0.98, 1.02))]), Some(false));
        // One pair above and one below: NO AGREEMENT.
        assert_eq!(
            cross_side(&[p((0.93, 0.97), (0.98, 1.02)), p((0.70, 0.80), (0.98, 1.02))]),
            None
        );
        // One pair straddles 0.9.
        assert_eq!(cross_side(&[p((0.85, 0.97), (0.98, 1.02))]), None);
    }
    /// V15-D18: the linux-a64 MEMREAD run of the exit-inputs job (fixture mem-linux-arm64.json:
    /// no d8 rows, jsvu has no linux-arm64 V8) passes on the node cap, and its text says the d8
    /// half is N/A by design. The node cap still gates there; on another host no d8 row is OPEN.
    #[test]
    fn item4_linux_a64_reads_the_node_cap_only() {
        let r = repo("mem-la64");
        let o = |f: PathBuf, host: &str| Options {
            mem: Some(f),
            ..opts(&r, host)
        };
        let c = &run(&o(input(&r, "mem-linux-arm64.json"), LINUX_A64))
            .unwrap()
            .cells[3];
        assert_eq!(c.status, Status::Pass, "{}", c.evidence);
        assert!(
            c.evidence.contains("vs node-jitless (cap 1.2): largest")
                && !c.evidence.contains("vs d8-jitless")
                && c.evidence.contains("; the d8 --jitless half: N/A by design (no linux-arm64 V8 in jsvu, DL:5351; V15-D18)"),
            "{}",
            c.evidence
        );
        let base = fixture("mem-linux-arm64.json").replace(SHA, &r.head);
        let over = base.replacen(
            "{ \"workload\": \"richards\", \"engine\": \"node-jitless\", \"n\": 5, \"median_bytes\": 50000000,",
            "{ \"workload\": \"richards\", \"engine\": \"node-jitless\", \"n\": 5, \"median_bytes\": 30000000,",
            1,
        );
        assert_ne!(over, base);
        write(&r.root, "inputs/mem-la64-over.json", &over);
        let c = &run(&o(r.root.join("inputs/mem-la64-over.json"), LINUX_A64))
            .unwrap()
            .cells[3];
        assert_eq!(c.status, Status::Fail, "{}", c.evidence);
        assert!(
            c.evidence.contains("x node-jitless (cap 1.2)"),
            "{}",
            c.evidence
        );
        let mac = base.replace(
            "\"platform\": \"linux-arm64\"",
            "\"platform\": \"darwin-arm64\"",
        );
        write(&r.root, "inputs/mem-mac-no-d8.json", &mac);
        let c = &run(&o(r.root.join("inputs/mem-mac-no-d8.json"), HOST))
            .unwrap()
            .cells[3];
        assert_eq!(c.status, Status::Open, "{}", c.evidence);
        assert!(
            c.evidence
                .contains("no d8-jitless row with a peak RSS for richards"),
            "{}",
            c.evidence
        );
    }

    /// The run id and slot fixtures (EI2-R1, EI2-R3-LEG-SLOT; made by
    /// `.github/scripts/exit-inputs.test.mjs`): the windows compare pathb-run-win.json and a
    /// Linux x64 compare `linux`, on x86_64-pc-windows-msvc. Each speed cell's form and text.
    fn run_cross(r: &Repo, linux: &str) -> Vec<(Option<HostForm>, String)> {
        let o = Options {
            pathb: Some(input(r, "pathb-run-win.json")),
            linux_pathb: Some(input(r, linux)),
            ..opts(r, WINDOWS_X64)
        };
        let rep = run(&o).unwrap();
        SPEED
            .iter()
            .map(|&i| (rep.forms[i], rep.cells[i].evidence.clone()))
            .collect()
    }

    /// EI2-R1, EI2-R3-LEG-SLOT: the legs of one GitHub run (1001), each pathb slot once: the
    /// cross form reads as before (AGREE above) and its text names the run.
    #[test]
    fn windows_cross_form_reads_one_run_and_each_slot_once() {
        let r = repo("pathb-run-one");
        for (form, ev) in run_cross(&r, "pathb-run-linux.json") {
            assert_eq!(form, Some(HostForm::AgreeAbove), "{ev}");
            assert!(
                ev.ends_with(
                    "; each pathb slot once (host.toml ids); one GitHub run 1001 (attempt 1)"
                ),
                "{ev}"
            );
        }
    }

    /// EI2-R1: the windows legs are of run 1001 and the Linux x64 legs of run 1002 (2
    /// dispatches): NO AGREEMENT, and the text names both run ids. With no same-model pair (the
    /// Linux legs of another CPU model), the cell does not fall back to the Path B form.
    #[test]
    fn windows_cross_form_with_two_run_ids_reads_no_agreement() {
        let r = repo("pathb-run-two");
        let want = "the legs are not of one exit-inputs dispatch with each pathb slot once (the legs are of 2 GitHub runs, not one (run 1001: w2, w1, w3, w4; run 1002: l1, l2)), so the same-model pairs are not known";
        for (form, ev) in run_cross(&r, "pathb-run-linux-2.json") {
            assert_eq!(form, Some(HostForm::NoAgreement), "{ev}");
            assert!(ev.contains(want), "{ev}");
        }
        let m = "AMD EPYC 7763 64-Core Processor";
        let other = fixture("pathb-run-linux-2.json")
            .replace(SHA, &r.head)
            .replace(m, "AMD EPYC 9V74 80-Core Processor");
        write(&r.root, "inputs/run-2-other.json", &other);
        let o = Options {
            pathb: Some(input(&r, "pathb-run-win.json")),
            linux_pathb: Some(r.root.join("inputs/run-2-other.json")),
            ..opts(&r, WINDOWS_X64)
        };
        let rep = run(&o).unwrap();
        for i in SPEED {
            let ev = &rep.cells[i].evidence;
            assert_eq!(rep.forms[i], Some(HostForm::NoAgreement), "{ev}");
            assert!(
                ev.contains(want) && !ev.contains("keeps the Path B form"),
                "{ev}"
            );
        }
    }

    /// EI2-R1: some legs name a run id and some do not (l2 of pathb-run-linux-mixed.json; or a
    /// windows compare of the older reader, with no ids, and a Linux x64 compare with ids): NO
    /// AGREEMENT, and the text names the legs with no run id.
    #[test]
    fn windows_cross_form_with_a_mixed_run_id_reads_no_agreement() {
        let r = repo("pathb-run-mixed");
        for (form, ev) in run_cross(&r, "pathb-run-linux-mixed.json") {
            assert_eq!(form, Some(HostForm::NoAgreement), "{ev}");
            assert!(
                ev.contains("(some legs name a GitHub run id and some do not (no run id: l2))"),
                "{ev}"
            );
        }
        let o = Options {
            pathb: Some(input(&r, "pathb-win.json")),
            linux_pathb: Some(input(&r, "pathb-run-linux.json")),
            ..opts(&r, WINDOWS_X64)
        };
        let rep = run(&o).unwrap();
        for i in SPEED {
            let ev = &rep.cells[i].evidence;
            assert_eq!(rep.forms[i], Some(HostForm::NoAgreement), "{ev}");
            assert!(
                ev.contains("win32-x64 leg w2: no host.toml id")
                    && ev.contains(
                        "some legs name a GitHub run id and some do not (no run id: w2, w1, w3, w4)"
                    ),
                "{ev}"
            );
        }
    }

    /// EI2-R3-LEG-SLOT: l1 and l2 both have the host.toml id gha-pathb-l1 (one artifact under 2
    /// dir names, or a doubled slot): the leg names differ, but the slot gha-pathb-l2 is missing
    /// and gha-pathb-l1 is given twice: NO AGREEMENT, and the text names the ids.
    #[test]
    fn windows_cross_form_with_a_doubled_slot_reads_no_agreement() {
        let r = repo("pathb-run-doubled");
        for (form, ev) in run_cross(&r, "pathb-run-linux-doubled.json") {
            assert_eq!(form, Some(HostForm::NoAgreement), "{ev}");
            assert!(
                ev.contains("(linux-x64: no leg has the id gha-pathb-l2; linux-x64: the id gha-pathb-l1 is given 2 times (legs l1, l2))"),
                "{ev}"
            );
        }
    }

    /// EI2-R3-LEG-SLOT: l2 has the host.toml id gha-pathb-l3: the slot gha-pathb-l2 is missing
    /// and gha-pathb-l3 is not a slot of the matrix: NO AGREEMENT, and the text names both.
    #[test]
    fn windows_cross_form_with_a_missing_slot_reads_no_agreement() {
        let r = repo("pathb-run-missing");
        for (form, ev) in run_cross(&r, "pathb-run-linux-missing.json") {
            assert_eq!(form, Some(HostForm::NoAgreement), "{ev}");
            assert!(
                ev.contains("(linux-x64: no leg has the id gha-pathb-l2; linux-x64 leg l2: the id \"gha-pathb-l3\" is not a pathb slot of the matrix)"),
                "{ev}"
            );
        }
    }

    /// EI2-R1: no leg names a run id (an older input: the fixtures of the older reader, or the
    /// legs of a dispatch before the run id field, which name their slot ids): the cross form
    /// reads as before, and the text has a note only.
    #[test]
    fn windows_cross_form_with_no_run_id_reads_as_before_with_a_note() {
        let r = repo("pathb-run-older");
        let note =
            "note: no leg names a GitHub run id (an older input), so one dispatch is not checked";
        let rep = run(&win_cross(&r, "pathb-gha-linux.json")).unwrap();
        for i in SPEED {
            let ev = &rep.cells[i].evidence;
            assert_eq!(rep.forms[i], Some(HostForm::AgreeAbove), "{ev}");
            assert!(
                ev.ends_with(&format!("; note: no leg names its host.toml id (an older input), so the slots are not checked; {note}")),
                "{ev}"
            );
        }
        let cut = "\n   \"run_id\": \"1001\",\n   \"run_attempt\": \"1\",";
        for f in ["pathb-run-win.json", "pathb-run-linux.json"] {
            let text = fixture(f).replace(SHA, &r.head);
            assert!(text.contains(cut), "{f}");
            write(
                &r.root,
                &format!("inputs/older-{f}"),
                &text.replace(cut, ""),
            );
        }
        let o = Options {
            pathb: Some(r.root.join("inputs/older-pathb-run-win.json")),
            linux_pathb: Some(r.root.join("inputs/older-pathb-run-linux.json")),
            ..opts(&r, WINDOWS_X64)
        };
        let rep = run(&o).unwrap();
        for i in SPEED {
            let ev = &rep.cells[i].evidence;
            assert_eq!(rep.forms[i], Some(HostForm::AgreeAbove), "{ev}");
            assert!(
                ev.ends_with(&format!("; each pathb slot once (host.toml ids); {note}")),
                "{ev}"
            );
        }
    }
}
