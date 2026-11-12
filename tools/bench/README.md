# tools/bench

Benchmark tools ([10-quality](../../docs/plan/10-quality.md) section 2; perf CI v0 of
[09-milestones](../../docs/plan/09-milestones.md) section 8 week 1).

| Path | What it does |
|---|---|
| `src/` (package `ep9_bench`, bin `ep9-bench`) | Runs a command N times. Writes the instructions retired per run as JSON (schema `ep9-bench/v1`) |
| `../../.github/scripts/perf-ci-v0.mjs` | perf CI v0: counts the T0 kernels, ci-v0, richards and deltablue under one counter; one JSON (schema `ep9-perf-ci/v1`) and a Markdown table |
| `jetstream/node-host.mjs` | JSC-shell host API on node, so JetStream's `cli.js` runs under node |
| `jetstream/lists.mjs` | Writes `expectations/jetstream-m2.txt` and `jetstream-m7.txt` from the pinned JetStream |
| `jetstream/run.mjs` | Runs JetStream workloads, one process each, under several configs; prints median scores |
| `jetstream/ep9-host.mjs` | The JSC-shell host API for an ep9 realm: the JetStream driver runs on node and each workload realm on ep9 (M2 exit 7) |
| `cross/run.mjs` | Cross-engine runner: ep9 against QuickJS, node `--jitless`, d8 `--jitless` and Brimstone. Wall time in paired rounds (class M) and instruction counts (class R). Writes ep9-cross/v1 JSON and Markdown |
| `cross/v8v7.mjs` | Generates a V8 suite v7 kernel with the real `base.js` (the default since MATH1, V9-D14) or a stub harness (`--harness stub`) from the pinned `third_party/bench-v8v7`; `--blockers` shows which kernels an engine runs |
| `cross/octane.mjs` | Generates an Octane-7 kernel with the real Octane `base.js` and a fixed iteration count from the pinned `third_party/bench-octane`; `--blockers` as in v8v7.mjs; `--kernel typescript` = the TS workload (3 files, N = 5) |
| `cross/kernels.mjs` | M1 exit 1: `--check ENGINE` runs the 15 kernels (8 v8-v7 with the default harness `base`, Octane-7) once on node and on ep9, compares the output, names the blockers, writes ep9-kernels/v1 JSON for `cargo xtask m1-exit --kernels` (item 1 passes at 15 of 15 on a fresh tree) |
| `cross/exit2.mjs` | M1 exit 2: reads an ep9-cross/v1 run of the `exit2` set; Octane-7 geomean vs QuickJS and v8-v7 7-kernel geomean vs node `--jitless`, with intervals |
| `cross/m2gate.mjs` | M2 exit 1, 2, 3, 6, 10, D11: reads the ep9-cross/v1 runs of the jit preset and the jitless build; writes ep9-m2gate/v1 for `cargo xtask m2-exit --gate` |
| `cross/mem.mjs` | M2 exit 4 (G7): median peak RSS of ep9 (both builds), node `--jitless` and d8 `--jitless` on the 7 v8-v7 kernels, >= 5 runs; both caps; writes ep9-mem/v1 for `cargo xtask m2-exit --mem` |
| `cross/workloads.mjs` | The cross workloads: T0 kernels from `workloads/`, ci-v0, the 8 v8-v7 kernels, Octane-7 (`octane-<kernel>`), `octane-typescript`, and the sets `v8v7-8`, `v8v7-7`, `octane-7`, `exit2`, `ts`, `m2` |
| `cross/idle.mjs` | Measures a host's idle baseline for its host file |
| `lib/stats.mjs` | PERF-NOISE statistics: median, order-statistic interval, verdict, IQR void rule, geomean. Shared by `cross/run.mjs` and `workloads/t0-dispatch.mjs` |
| `lib/host.mjs` | Host facts, host files, the load guard; `thpMode`, the transparent huge page mode of a Linux host (the bracketed word of `/sys/kernel/mm/transparent_hugepage/enabled`, null elsewhere), which `cross/run.mjs` writes to `host.measured` and `cross/exit2.mjs` copies for m1-exit item 2 |
| `hosts/<host>.toml` | One file per benchmark host: CPU, cores, OS, gate host or not, idle baseline |

## ep9-bench

```bash
cargo run --release -p ep9_bench -- --runs 5 --out out.json -- target/release/ep9 -e 0
```

| Host | Counter (`--counter auto`) | Source of the count |
|---|---|---|
| macOS | `time` | `/usr/bin/time -l`, line "instructions retired" |
| macOS VM without PMC access (hosted macOS runner) | none (exit 3) | `time -l` prints no "instructions retired" line |
| Linux | `perf` if it gives a count | `perf stat -x, -e instructions:u` |
| Linux, no hardware counters (many VMs) | `cachegrind` | `valgrind --tool=cachegrind`, line "I refs" |
| Linux with neither, Windows | none (exit 3) | none |

- Before the runs, ep9-bench probes the counter: `true` under it must give a count above 0. Apple's `time.c` prints "instructions retired" only for a count above 0, so a VM without PMC access gives no line.
- No counter: ep9-bench still runs the command (wall time only) and writes the JSON with `"counter": null`, `"counter_status": "unavailable"`, `"counter_note"` (why), and `"instructions": null` in each run and in the summary. It exits 3. A null count means "not measured". Never read it as 0.
- Exit status: 0 counts written; 1 error (the command failed, a counted run gave no count, or the JSON was not written); 2 usage error; 3 no counter (the JSON is still written); 4 a run printed the wrong output (`--expect-stdout`; no JSON).
- The JSON names the counter. Do not compare counts from different counters.
- A nonzero exit of the command is an error. The command's output is discarded unless `--show-output`;
  the error of a failed run then ends with the last 20 lines of the command's stderr.
- `--expect-stdout FILE`: every run, warm-up and measured, must print what FILE holds, compared after CRLF -> LF and a trim of leading and trailing whitespace (the refusal rule of `cross/run.mjs`). The first run that prints anything else stops ep9-bench with exit 4 and its output in the error; no JSON is written, so a wrong answer never gets a count. The counter's own report goes to its own file, so this works under every counter. Not with `--show-output`.
- `commit` comes from `GITHUB_SHA` or `EP9_COMMIT`.
- `EP9_BENCH_TIME` replaces `/usr/bin/time`. `tests/testdata/time-no-pmc.sh` is a fake that prints what a host with no PMC access prints.
- Tests: `cargo test -p ep9_bench`. The tests probe the OS tool themselves (not through ep9-bench). With a counter, `counts_instructions` checks the counts. Without one, it prints `SKIPPED the count check` and checks the no-counter JSON instead. On macOS, `EP9_BENCH_TIME=$PWD/tools/bench/tests/testdata/time-no-pmc.sh cargo test -p ep9_bench` runs that path. `tests/perf_ci.rs` runs perf-ci-v0.mjs with node (`EP9_NODE`; it prints SKIPPED without node) and the fake engine `tests/testdata/right-then-wrong.sh`, which is right in its first `RIGHT_RUNS` runs only.

## perf CI v0

The CI job `perf-ci-v0` (Linux x64) records instruction counts per commit. It is a record, not a
gate yet. The same commands run on any Linux host with valgrind and node:

```bash
sudo apt-get install -y valgrind
bash scripts/sync-bench.sh bench-v8v7          # richards and deltablue need the v8-v7 pin
cargo build --release --locked -p ep9_bench -p ep9_cli
node .github/scripts/perf-ci-v0.mjs --ep9 target/release/ep9 --bench target/release/ep9-bench \
  --counter cachegrind --runs 3 --warmup 0 --out perf-ci-v0.json --md summary.md
```

```text
workload                               file                                     check, then count
t0-loop t0-branchy t0-call t0-prop     tools/bench/workloads/<name>.js           output = the `want`
ci-v0                                  tools/bench/workloads/ci-v0.js            of workloads.mjs
richards deltablue                     cross/v8v7.mjs --kernel <name> (pinned)  "<name> <N>" lines
          |
          v
one ep9 run, no counter: exit 0 and the expected output, else "refused" (no count)
          |
          v
ep9-bench --counter cachegrind --runs 3 --warmup 0 --expect-stdout <expected output>,
EP9_HASH_SEED=1, workloads in parallel. A run with another output: exit 4, "refused" (no count)
          |
          v
perf-ci-v0.json (schema ep9-perf-ci/v1) + a Markdown table (--md appends; CI: the step summary)
```

- **Counter.** Always cachegrind (`--counter cachegrind`), never perf: cachegrind counts user-mode instructions on any VM, and with a fixed hash seed it gives the same count on every run. Compare counts only within one counter and one target.
- **Hash seed.** The string hash seed is random per process, and it changes how far shape-index probes go. Without a fixed seed, 3 cachegrind runs of t0-call spread by 3.7%; with `EP9_HASH_SEED=1` (the script's default, `--hash-seed`) they spread by 609 instructions in 6.1e9. So a re-run on the same commit agrees to well within 0.1%.
- **Runs.** 3 runs and no warm-up: with the fixed seed, more runs add time, not information. The spread column shows (max - min) / median.
- **JSON.** `commit` (`GITHUB_SHA`, else `EP9_COMMIT`, else git HEAD), `counter`, `target`, `runs`, `warmup`, `hash_seed`, `host`, `complete`, and per workload: `name`, `status` (`ok`, `refused` or `error`), `error`, `instructions` (`min`, `median`, `max`, or null), the file and its `sha256`, the expected output, the v8-v7 `pin`, `blob` and `n`, and the whole ep9-bench/v1 report (`bench`).
- **Refusal.** Every run checks the output, the counted ones too: ep9-bench gets the expected output (`--expect-stdout`), so an engine that is right in the check run and wrong in a counted run is refused, not counted (W4-PERF-REFUSE).
- **Exit status.** 0 every workload has a count; 1 a workload was refused or has no count (the JSON and the table are still written, with null counts); 2 usage error.
- `--workloads` picks a subset; `--jobs N` limits the parallel workloads (default: the CPU count); `--work DIR` keeps the generated v8-v7 files.

## Bench mode

BENCH-SET on GitHub (plan v13 rule 13 and section 3.2; decision M8-PLAN-D18). The rule-13
floor of a role reads it: no row worse than +0.5% Ir, head against base, on linux-x64 and
linux-arm64. A dispatch of `ci.yml` with the input `bench_base` runs only the `scope` job and the
2 legs of the `bench-set` job; every other job, `lint` too, is skipped. The orchestrator runs it
(agents do not run BENCH-SET on the Mac):

```bash
gh workflow run ci.yml --ref <ref with this ci.yml> -f bench_base=<base sha> -f bench_head=<head sha>
# a subset of rows, or rows outside the BENCH-SET (here the TS workload and deltablue):
gh workflow run ci.yml --ref <ref with this ci.yml> -f bench_base=<base sha> -f bench_head=<head sha> \
  -f bench_workloads=octane-typescript,deltablue
```

```text
bench_base (full sha) ──┐                       per leg: linux-x64 (--branch-sim), linux-arm64
bench_head (full sha;   ├─> checkout both trees ─> cargo build --release -p ep9_cli in each
 empty: the dispatched  │   (each with its own toolchain file, regress and icu4x forks and
 commit)                │    its own target dir, bench/target-<side>), and the jitless build
                        │    of each: --no-default-features (bench/target-jitless-<side>);
                        │    then the head rebuild guard on the 2 head builds
dispatched commit ──────┘─> the scripts and the workload files (sync-bench.sh bench-v8v7 bench-octane)
          |
          v
.github/scripts/bench-set.mjs, twice per leg: --build jit (19 rows), then --build jitless
  (12 rows); per workload and side, one run
  valgrind --tool=cachegrind --cache-sim=no --branch-sim=<yes|no> <ep9> <file>
  EP9_HASH_SEED=1, EP9_GC_HELPERS=0; output = the expected output, else "no count"
          |
          v
bench-set-<leg>.json and bench-set-<leg>-jitless.json (schema ep9-bench-set/v1) + 2 tables in
the step summary; artifact bench-set-<leg>-<head sha>: both JSONs and the cachegrind .out and
.log files (bench/cg/, bench/cg-jitless/)
```

- **Rows.** The BENCH-SET of rule 13: t0-loop, t0-branchy, t0-call, t0-prop, t0-gen, the 7 v8-v7 kernels (no regexp) and Octane-7: 19 rows (`--workloads` takes other names and `workloads.mjs` set names). Both builds run the same workload text: it comes from the dispatched commit.
- **The jitless leg** (route MC-w36-4). The jit build (the `ep9_cli` default features: the `jit` feature, preset `jit`, feedback `full`) runs the recording handlers; the jitless build (`--no-default-features`: no `jit` feature, preset `jitless`) runs the `ics` handler table with `skip_next`, which the M2 gate D11 and the A3 FV1 target read. So each leg also builds both sides with `cargo build --release --locked -p ep9_cli --no-default-features` in its own target dir (`bench/target-jitless-<side>`, so the jit artifacts stay) and runs `bench-set.mjs --build jitless` on the JITLESS-SET: t0-loop, t0-branchy, t0-call, t0-prop, t0-gen and the 7 v8-v7 kernels, 12 rows (or the `bench_workloads` rows). Rule 13 is read on both builds: the `count` step runs both legs and fails when either fails. `--build jit|jitless` makes `bench-set.mjs` check each binary's build line (`features <list>; preset <name>`: jit needs the feature `jit` and preset `jit`, jitless neither; else exit 2) and names the build in the JSON (`build`) and the table title. Without `--build` there is no check and `build` is null.
- **`bench_workloads`.** The dispatch input for `--workloads`: a comma list of `workloads.mjs` names or set names, for example `octane-typescript,deltablue` (`octane-typescript` is the TS workload: the 3 files of the Octane `typescript` kernel, N = 5, with the same output check as the other rows). Empty: the 19 rows. It reaches the scripts through `env` only. The `bench mode` step refuses a value that is not a comma list of names (a-z, 0-9, `-`), and the `bench workloads` step (`bench-set.mjs --list`) refuses an unknown name; both exit 2, before the checkouts and the builds. The input with no `bench_base` is refused too. The concurrency group names it, so 2 dispatches of one pair with other workloads do not replace each other. The rule-13 floor of a role reads the 19 rows and the 12 jitless rows; a run with `bench_workloads` gives only the rows it names, in both legs.
- **Verdict.** Per row: `pass` when head Ir ≤ base Ir × 1.005, `FAIL` above it, `no count` when a side has none (a wrong output, a failed run, no count in the log). The compare is on integers, so +0.500% passes. The run is PASS (every row passes), FAIL (a row fails) or INCOMPLETE; the leg fails unless PASS. A row that is faster by any amount passes: rule 13 is a floor.
- **One run per side.** With a fixed seed and no GC helper threads, cachegrind gives the same count on every run (perf CI v0 above; the x86 fixer and S0 boxes), so the job counts each side once.
- **X86IR.** The linux-x64 leg runs with `--branch-sim=yes`: each row also has Bc, Bcm, Bi and Bim (indirect-branch mispredicts in the cachegrind model, a relative signal only). Every row lists the functions with the largest Ir change and the largest Ir (the out files, summed per function name over all source files; `--top`, default 15; the table shows the 10 largest changes). `--functions LIST` (substrings of function names) keeps those functions in the rows and in the table beside the cut, with `named: true`; the job passes `SetNamedProperty_cold,DefineNamedOwnProperty_cold` (rule 81: a `_cold` handler is judged by its X86IR rows). The .out files are in the artifact for `cg_annotate`.
- **Target dirs and the head rebuild guard** (route BENCH-STALE-HEAD; decisions log M6-ORCH-560). Base and head never share a target dir: `bench/target-base`, `bench/target-head`, `bench/target-jitless-base`, `bench/target-jitless-head`. Until M8-P2 both sides built in one dir; the checkouts came before the base build, so a head crate whose files were older than the base artifacts was fresh to cargo and the head binary kept base code (only the crates that depend on the re-synced regress and icu4x checkouts compiled again). The readings of a change in `ep9_vm` alone stay valid; a change below it (for example `ep9_frontend`, `ep9_heap`, `ep9_unit`) was not measured. Each head build writes its cargo JSON messages (`--message-format=json-render-diagnostics`) to `bench/build-head.jsonl` and `bench/build-jitless-head.jsonl`, and `bench-set.mjs --rebuild-guard --base-tree bench/base --head-tree bench/head --cargo-json <file>` fails the leg (exit 1, with the package names) when a package of the build whose directory differs from base (any file, `target` and `.git` not read) has a compiler artifact that cargo did not compile (`"fresh": true`; a build script's artifact is not read). Tests: `node --test .github/scripts/bench-set.test.mjs` (the guard and its CLI on fake trees and fake cargo messages).
- **Commits.** `bench_base` and `bench_head` must be full commit ids. Each binary's `ep9 --version` build line (P0-BUILDID) must name its commit, built clean; else the leg stops with exit 2.
- **JSON.** `target`, `build` (`jit`, `jitless`, or null with no `--build`), `counter` (`cachegrind-i-refs`), `branch_sim`, `hash_seed`, `gc_helpers`, `limit_pct`, `functions_named`, `valgrind`, `host`, `base` and `head` (`commit`; `ep9`: `version`, `engine_build`, `build`), `notes`, `verdict`, `worst` (per group t0, v8v7, octane: the row with the largest delta), and per row: `name`, `group`, the file's `sha256`, `expect` and source, `base` and `head` (`status`, `error`, `ir`, `bc`, `bcm`, `bi`, `bim`), `delta_pct`, `verdict`, `bim_delta_pct`, `out_rows_match` (the rows of each .out file sum to its summary line) and `functions`.
- **Push and pull_request.** The `bench-set` job has no `needs` and no job-level `if`, so it is never skipped (the tag gate refuses a skipped job): without `bench_base` its matrix is one leg, `off`, that prints "bench mode off" and runs `node --test .github/scripts/bench-set.test.mjs`.
- **Local.** On a Linux host with valgrind: build both binaries, then `node .github/scripts/bench-set.mjs --base-ep9 A --head-ep9 B --base <sha> --head <sha> --out bench-set.json` (`--branch-sim`, `--keep DIR`, `--jobs N`, `--limit-pct`).

## JetStream

Needs `bash scripts/sync-bench.sh` first (`bash scripts/sync-bench.sh bench-jetstream` syncs only
JetStream; with no argument it syncs all three bench pins).

```bash
node tools/bench/jetstream/lists.mjs            # write the M2 and M7 lists
node tools/bench/jetstream/lists.mjs --check    # exit 1 if they are stale
node tools/bench/jetstream/run.mjs --list expectations/jetstream-m7.txt --reps 7 \
  --out out.json --config full=node --config maxopt2=node:--max-opt=2
```

- The host gives each workload a fresh global from `vm.createContext(vm.constants.DONT_CONTEXTIFY)`: a plain V8 global with no node interceptors, like a d8 `Realm`.
- A shell without WebAssembly (for example `node --jitless`) needs `cli.js --no-prefetch`, which reads the files that `sync-bench.sh` inflates. In `run.mjs`, a config flag `cli:<arg>` goes to `cli.js`: `--config jitless=node:--jitless,cli:--no-prefetch`.
- Each workload needs `--reps` scores in every config. If a run fails or gives no score, `run.mjs` still writes the JSON, but sets `"complete": false`, lists the short pairs in `"missing"`, sets the summary scores and ratios to `null`, and exits 1. Never quote a ratio from an incomplete run.
- `EP9_JETSTREAM_DIR` overrides the checkout directory.
- Tests (fake engine, no checkout needed, about 2 s): `node --test tools/bench/jetstream/run.test.mjs`.

## Cross-engine runner

Needs `bash scripts/sync-shells.sh` (the reference shells) and `bash scripts/sync-bench.sh
bench-v8v7` (for richards), and a release build: `cargo build --release --locked -p ep9_cli -p ep9_bench`.

```bash
node tools/bench/cross/run.mjs --ep9 target/release/ep9 --host tools/bench/hosts/m5max-macos.toml \
  --count target/release/ep9-bench --out run.json --md run.md
node tools/bench/cross/v8v7.mjs --kernel richards --out richards.js   # N = 200; same file every time
node tools/bench/cross/v8v7.mjs --blockers target/release/ep9         # which v8-v7 kernels run
node tools/bench/cross/octane.mjs --blockers target/release/ep9       # which Octane-7 kernels run
node tools/bench/cross/idle.mjs                                        # idle baseline, 10 minutes
```

`--workloads` takes workload names and set names: `v8v7-8` (the 8 v8-v7 kernels), `v8v7-7` (no
regexp), `octane-7`, `exit2` (`v8v7-7` + `octane-7`). The default harness of the v8-v7 kernels is
`DEFAULT_HARNESS` in `cross/v8v7.mjs` (one line, `cross/v8v7.mjs:80`): `base` since MATH1
(5495858, M4-P1; plan v9 V9-D14: exits 1 and 2 use the real base.js), `stub` before; `--harness
stub` keeps the runs 1 and 2 form. It is also the default of `kernels.mjs` and of perf CI v0.

The rules come from the PERF-NOISE rule ([10-quality](../../docs/plan/10-quality.md) section 3):

- **Order.** A round runs every workload, and an empty script, on every engine. The workload order rotates by one per round. Per workload, the engine order rotates by one per round from a start that differs per workload, so over E rounds each engine runs in each position once.
- **Rounds.** At least 10 measured rounds (class M), plus 1 warm-up round that is dropped.
- **Ratio.** Per round, `t(engine) / t(ep9)`: ep9's speed relative to that engine (> 1: ep9 faster). Net time is a run's time minus the same engine's empty-script time in the same round; the verdict uses net ratios, and gross ratios are shown too. The empty script is the `print` prelude alone (node has no `print`; the prelude maps it to `console.log`).
- **Statistics.** The median of the per-round ratios, the order-statistic interval (n = 10: ranks 2 and 9), and the verdict: "ep9 faster" or "ep9 slower" when the whole interval is on one side of 1, else "marginal". A row is void when the IQR of its ratios is more than 5% of the median.
- **M0 exit 4 canary.** richards and deltablue against QuickJS, with 0.6 as the threshold: "pass", "fail" or "marginal", but only for a run that is not provisional.
- **Load guard.** With `--host`: 60 s with no benchmark process, then the 1-minute load must be at most the idle baseline + 0.5. The runner waits up to 30 min for that, then stops with "host busy" (exit 3), unless `--allow-busy`. 120 s after the last round it checks again. The JSON keeps both values and the load after each round.
- **Provisional.** The run is provisional when there is no host file, the host is not a gate host, the host file has no idle baseline, or the load guard failed. A void row is provisional too, and so is a row with a net time of 0 or less in any round ("net invalid"; its gross ratio is still shown, marked "(gross)"). A provisional result never passes or fails a gate.
- **Refusal.** Every run must exit with 0 and print the workload's expected output (the T0 kernels' `want`; for v8v7 kernels, one `<name> <N>` line per benchmark). Otherwise the runner prints `REFUSED`, writes no report and exits 1. Before any timing, it runs every file once on every engine to check this.
- **Class R.** With `--count`, ep9-bench counts every R workload (all of them; ci-v0 is R only) on every engine, 5 runs after 1 warm-up, and keeps the median. Every counted run's output is checked (`ep9-bench --expect-stdout`; a wrong answer is REFUSED), and ep9's counted runs get `EP9_HASH_SEED=1` (`--hash-seed`). On macOS `time -l` counts the whole process (user and kernel); on Linux cachegrind counts user mode only. Compare counts only within one counter.
- Exit status: 0 done; 1 refused or error; 2 usage error; 3 host busy.
- Tests (fake engines in `cross/testdata/`, no shells or ep9 build needed; the v8-v7, Octane and kernels tests need the bench checkouts and skip without them; 66 s on the M5 Max at load 9-10, 2026-10-26): `node --test tools/bench/cross/run.test.mjs`.

### The v8-v7 stub harness

The official harness (`base.js` + `run_harness.js`) needs `Date` for its timer and score, and `Math`.
When the stub was made (decision PERF-2), ep9 T0 had no `Date` and no `Math.log` or `Math.pow`. `Date` came
in M3-P1 and those two in M4-P1 (MATH1), so `base` is the default harness since then (next section).
The stub `cross/v8v7.mjs` keeps the kernel file as it is and replaces only the
harness: `BenchmarkSuite` and `Benchmark` record the benchmarks, each benchmark runs setup, then
`run()` N times, then tear-down, and prints `<name> <N>`. `Math.random` is base.js's deterministic
version where `Math` exists. The kernels check their own results. The generator reads the kernel
from the pinned commit (`third_party/bench-v8v7.commit`), refuses a checkout at another commit or
with a changed file, and writes no date, so two runs give the same file. Its first lines name the
pin and the kernel's blob id. Default N: richards 200; the others give QuickJS about the same time
(`--list`).

### The v8-v7 base harness (run 3)

`--harness base` (v8v7.mjs, run.mjs and kernels.mjs; the default is `DEFAULT_HARNESS`) runs the real pinned `base.js`,
unchanged, as its own strict script (an indirect eval, as d8 loads a separate file), then the
kernel (sloppy, as in d8), then base.js's own `BenchmarkSuite.RunSuites`. The driver replaces one
method only: `RunSingleBenchmark` runs `run()` N times (timed with `Date`) instead of "for 1 s",
because class M compares wall time per process. Setup, tear-down, `NotifyResult`,
`GeometricMean` and the score code are base.js's. The output is the stub's (`<name> <N>`); an
error prints an `ERROR` line, so the runner refuses the run. It needs `Date`, `Math.log`,
`Math.pow` and `Number.prototype.toFixed`; `--blockers ENGINE --harness base` shows which kernels
run. The JSON names the harness per workload.

### Octane-7

Octane-7 = richards, deltablue, crypto, raytrace, earley-boyer, splay, navier-stokes from the
pinned Octane 2 (`third_party/bench-octane`, `bash scripts/sync-bench.sh bench-octane`); regexp is
not in it. `cross/octane.mjs` writes one file: the print prelude, the pinned `base.js` unchanged,
the pinned kernel unchanged (d8's load order; both are sloppy scripts with no top-level `let`,
`const` or `class`, so one script runs the same code as two), then a driver. The driver changes
data only: `BenchmarkSuite.config` turns on base.js's deterministic mode with no warm-up, and
each benchmark's `deterministicIterations` and `minIterations` are set to N. So base.js's own
`RunSuites`, `RunSingleBenchmark` (timed with `Date`), setup, tear-down, scores
(`GeometricMeanTime`: `Math.log`, `Math.pow`) and splay's latency score all run, on exactly N
`run()` calls. It prints `<name> <N>` per benchmark; an error, or a score that is not a number
above 0, prints an `ERROR` line, so the runner refuses the run. N = the v8v7.mjs N of the same
kernel (`--list`). Pin check and refusal rule as in v8v7.mjs. In the cross JSON its harness is
`octane-base`.

### M1 exits 1 and 2

```bash
node tools/bench/cross/kernels.mjs --check target/release/ep9 --out kernels.json   # exit 1
node tools/bench/cross/run.mjs --ep9 target/release/ep9 --host tools/bench/hosts/m5max-macos.toml \
  --engines ep9,qjs,node-jitless --workloads exit2 --harness base --out run.json --md run.md
node tools/bench/cross/exit2.mjs run.json --out exit2.json                          # exit 2
```

- **kernels.mjs** runs each of the 15 kernels (ids `v8v7/<k>` and `octane/<k>`) once (N = 1) on
  node (the oracle; `--node`, default the node that runs it) and on ep9. A kernel is
  "runs, output equal" when ep9 exits with 0 and prints what node prints, and node must print
  the generator's `<name> <N>` lines (else exit 3, no verdict). Those lines come from the
  harness, so the kernels that check no result of their own (v8-v7 regexp; navier-stokes, whose
  Octane check runs only at frame 15) get a result probe in the checked file: after each run()
  it prints `check <name> <i> <hash>`, a hash of the regexp results or of the solver's grids,
  which ep9 must print as node does (timed runs have no probe). The hash names each value's
  type, so NaN, an infinity, undefined or a number in place of a string never hashes as node's
  value; a grid cell counts at a 1e-9 grain with no 32-bit wrap. A failure shows the first error,
  its line in base.js or the kernel file, and a named blocker when the error names a feature
  (pow core, RegExp, or the `Math.<f>` calls of that line that ep9's `Math` lacks).
  `--not-yet LIST` names kernels that may fail (a role's NOT-YET list). The JSON (schema
  `ep9-kernels/v1`) has the commit, the git tree ids of `crates`, `tools/bench`, the root
  `Cargo.toml`, `Cargo.lock` and `rust-toolchain.toml`, the paths among them with uncommitted
  changes (`dirty`), the host target triple, the ep9 binary's SHA-256 and every kernel's result. Exit 0 every kernel ran
  (those on `--not-yet` may fail); 1 a kernel failed; 2 usage; 3 no verdict.
- **exit2.mjs** reads one ep9-cross/v1 run. Per round, the geomean of the kernels' net ratios
  is one paired value (each round runs every kernel), so a reading is the median of the per-round
  geomeans with the order-statistic interval and the IQR void rule. Gate: "pass" when the
  interval's low end is at least the threshold (Octane-7 vs QuickJS 1.0; v8-v7 7-kernel vs node
  `--jitless` 0.75), "fail" when its high end is below it, else "marginal". No gate verdict when
  the run is provisional, a kernel row is not valid, the per-round geomeans are void, a v8-v7
  kernel ran without the real base.js (V9-D14), or the run has fewer than 10 paired rounds
  (`M.rounds`, `options.rounds` and every ratio series must agree). Freshness is m1-exit's check (the JSON names the
  commit). Exit 0 both pass; 1 otherwise; 2 usage.
- Tests: `node --test tools/bench/cross/run.test.mjs` (fixtures in `cross/testdata/`:
  `cross-exit2.json`, a synthetic run; `fake-kernels-ep9.sh`, an engine that runs node or fails
  on purpose; `fake-fault.mjs`, its faults for the probe tests).

### M2 exit readings (the M2 reader)

```bash
# Once per build: the jit preset (default features) and the jitless build (--no-default-features).
node tools/bench/cross/run.mjs --ep9 target/release/ep9 --host tools/bench/hosts/m5max-macos.toml \
  --engines ep9,qjs,node-jitless --workloads m2 --harness base --count target/release/ep9-bench \
  --out run-jit.json --md run-jit.md
node tools/bench/cross/m2gate.mjs run-jit.json run-jitless.json --out m2gate.json   # items 1-3, 6, 10, D11
node tools/bench/cross/mem.mjs --ep9 target/release/ep9 --ep9-jitless <jitless ep9> \
  --host tools/bench/hosts/m5max-macos.toml --out mem.json --md mem.md               # item 4
node tools/bench/jetstream/run.mjs --list expectations/jetstream-m2.txt --config ep9=ep9@target/release/ep9 \
  --config jitless=node:--jitless,cli:--no-prefetch --out jetstream.json             # item 7
cargo xtask m2-exit --host aarch64-apple-darwin --gate m2gate.json --mem mem.json \
  --exec test262-exec.json --fastdiff fastdiff.json --jetstream jetstream.json \
  --fuzz <record>... --p-gates docs/measurements/<date>-p-gates.md --out m2-exit.md
```

- **The TS workload** (M2 exit 3, plan v12 V12-D5) is `octane-typescript`: `octane.mjs --kernel
  typescript` puts the pinned base.js and the 3 files `typescript.js`, `typescript-input.js`,
  `typescript-compiler.js` (in the pinned run.js order) into one script, with the benchmark's own
  5 deterministic iterations and no warm-up; it prints `Typescript 5`. The set `ts` runs it alone
  (TSREAD: `run.mjs --workloads ts`); a refused run names the part file of the first error line.
  The set `m2` is `v8v7-7` + `octane-7` + `octane-typescript`.
- **m2gate.mjs** reads the ep9-cross/v1 run of each build (the build comes from ep9's build line:
  preset `jit` with the feature `jit`, or preset `jitless` without it; two runs of one build or
  of two hosts are refused) and writes ep9-m2gate/v1: per build, `octane7-vs-qjs` (threshold
  1.33), `v8v7-7-vs-node-jitless` (1.0; each kernel not proven >= 0.8 is listed as
  `parity_debt`), `splay-vs-node-jitless` (1.0), all with exit2.mjs's per-round geomean and
  interval rule, and `ts`: the median gross ep9 time with its interval, the net time, and the
  paired ratios to node `--jitless` and QuickJS. On m5max-macos `ts` passes when the upper end of
  the gross interval is <= 2.8 s; on Linux x64 at >= 1.0x node `--jitless`; on another host it
  has no rule (no gate verdict, but the status is `valid` when the measurement is). `ts` needs a
  valid net ratio to both comparators. `pass` needs every reading of both builds (V12-D6). Exit
  0 pass; 1 not; 2 usage.
- **cargo xtask m2-exit** (`tools/xtask/src/m2_exit.rs`) gives one column per host: items 1-10
  and a D11 row. The speed cells read m2gate.json (both builds), check the host platform, the
  build line and that the commit is fresh. A Linux x64 speed cell reads "marginal (no gate host)"
  without a gate host; a Windows x64 speed cell gives the ratio to the Linux x64 run
  (`--linux-gate`; each reading on both hosts must have the status `valid`) and reads "marginal
  (no gate host)" until a dual-boot or identical gate host exists. Item 4 reads mem.json (ep9-mem/v1), item 5 the test262 exec JSON by directory (G1 file
  rule) and the FASTDIFF JSON (ep9-fastdiff/v1; a fast path with 0 cases is OPEN), item 7 the
  JetStream run, item 8 the ep9-fuzz/v1 records (Fuzzilli >= 500 counted CPU-h, 0 open crashes),
  item 9 the P-gate file. The module doc lists the fields each input needs; the fixtures are in
  `tools/xtask/tests/m2_exit/`. Read the cell lines and the Evidence lines, not the exit code.
- The tree readers (`adr-lint`, `m0-exit`, `m1-exit`, `m2-exit`) read the git top dir of the
  current dir unless `--root` is given, so one xtask build serves two worktrees.
- **mem.mjs** and **jetstream/run.mjs** (BENCH10): the usage is in the header comment of each
  file. mem.mjs writes ep9-mem/v1 (engine `ep9` is the jit preset build, `ep9-jitless` the
  jitless build); run.mjs writes the `ep9` block (`commit`, `dirty`) and the `kind` and
  `version` of each config that item 7 reads (an ep9 config is one of kind `ep9`). Config names
  must differ: run.mjs exits 2 on a repeated name, and item 7 reads such a file OPEN.
- Tests: `node --test tools/bench/cross/m2gate.test.mjs` (also loaded by `run.test.mjs`); the
  m2-exit fixtures `gate-*.json` are its output (`EP9_BLESS=1` writes them again);
  `cargo test --locked -p xtask` (m2-exit and the two-worktree test `tests/root_default.rs`).

### Host files

`hosts/<host>.toml` gives `id`, `cpu` (must equal what node reports on that host), `cores`,
`platform`, `os`, `gate_host`, and `idle_baseline_load1`: the median 1-minute load over 10 idle
minutes, from `node tools/bench/cross/idle.mjs` (it refuses a window in which builds or benchmarks
run). Without `idle_baseline_load1` the load guard cannot pass, so every run there is provisional.
`hosts/m5max-macos.toml` has `idle_baseline_load1 = 2.505`, measured on 2026-10-21 (10 idle minutes).

