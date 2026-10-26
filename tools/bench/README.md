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
| `cross/run.mjs` | Cross-engine runner: ep9 against QuickJS, node `--jitless`, d8 `--jitless` and Brimstone. Wall time in paired rounds (class M) and instruction counts (class R). Writes ep9-cross/v1 JSON and Markdown |
| `cross/v8v7.mjs` | Generates a V8 suite v7 kernel with a stub harness (or the real `base.js`, `--harness base`) from the pinned `third_party/bench-v8v7`; `--blockers` shows which kernels an engine runs |
| `cross/octane.mjs` | Generates an Octane-7 kernel with the real Octane `base.js` and a fixed iteration count from the pinned `third_party/bench-octane`; `--blockers` as in v8v7.mjs |
| `cross/kernels.mjs` | M1 exit 1: `--check ENGINE` runs the 15 kernels (8 v8-v7, Octane-7) once on node and on ep9, compares the output, names the blockers, writes ep9-kernels/v1 JSON for `cargo xtask m1-exit --kernels` |
| `cross/exit2.mjs` | M1 exit 2: reads an ep9-cross/v1 run of the `exit2` set; Octane-7 geomean vs QuickJS and v8-v7 7-kernel geomean vs node `--jitless`, with intervals |
| `cross/workloads.mjs` | The cross workloads: T0 kernels from `workloads/`, ci-v0, the 8 v8-v7 kernels, Octane-7 (`octane-<kernel>`), and the sets `v8v7-8`, `v8v7-7`, `octane-7`, `exit2` |
| `cross/idle.mjs` | Measures a host's idle baseline for its host file |
| `lib/stats.mjs` | PERF-NOISE statistics: median, order-statistic interval, verdict, IQR void rule, geomean. Shared by `cross/run.mjs` and `workloads/t0-dispatch.mjs` |
| `lib/host.mjs` | Host facts, host files, the load guard |
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
`DEFAULT_HARNESS` in `cross/v8v7.mjs` (one line; `stub` until MATH1 sets `base`, plan v9 V9-D14);
it is also the default of `kernels.mjs` and of perf CI v0.

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
ep9 T0 has neither yet, so `cross/v8v7.mjs` keeps the kernel file as it is and replaces only the
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
  the generator's `<name> <N>` lines (else exit 3, no verdict). A failure shows the first error,
  its line in base.js or the kernel file, and a named blocker when the error names a feature
  (pow core, RegExp, or the `Math.<f>` calls of that line that ep9's `Math` lacks).
  `--not-yet LIST` names kernels that may fail (a role's NOT-YET list). The JSON (schema
  `ep9-kernels/v1`) has the commit, the git tree ids of `crates`, `tools/bench`, `Cargo.lock`
  and `rust-toolchain.toml`, the paths among them with uncommitted changes (`dirty`), the host
  target triple, the ep9 binary's SHA-256 and every kernel's result. Exit 0 every kernel ran
  (those on `--not-yet` may fail); 1 a kernel failed; 2 usage; 3 no verdict.
- **exit2.mjs** reads one ep9-cross/v1 run. Per round, the geomean of the kernels' net ratios
  is one paired value (each round runs every kernel), so a reading is the median of the per-round
  geomeans with the order-statistic interval and the IQR void rule. Gate: "pass" when the
  interval's low end is at least the threshold (Octane-7 vs QuickJS 1.0; v8-v7 7-kernel vs node
  `--jitless` 0.75), "fail" when its high end is below it, else "marginal". No gate verdict when
  the run is provisional, a kernel row is not valid, the per-round geomeans are void, or a v8-v7
  kernel ran without the real base.js (V9-D14). Freshness is m1-exit's check (the JSON names the
  commit). Exit 0 both pass; 1 otherwise; 2 usage.
- Tests: `node --test tools/bench/cross/run.test.mjs` (fixtures in `cross/testdata/`:
  `cross-exit2.json`, a synthetic run; `fake-kernels-ep9.sh`, an engine that runs node or fails
  on purpose).

### Host files

`hosts/<host>.toml` gives `id`, `cpu` (must equal what node reports on that host), `cores`,
`platform`, `os`, `gate_host`, and `idle_baseline_load1`: the median 1-minute load over 10 idle
minutes, from `node tools/bench/cross/idle.mjs` (it refuses a window in which builds or benchmarks
run). Without `idle_baseline_load1` the load guard cannot pass, so every run there is provisional.
`hosts/m5max-macos.toml` has `idle_baseline_load1 = 2.505`, measured on 2026-10-21 (10 idle minutes).

