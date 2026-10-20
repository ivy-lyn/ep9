# tools/bench

Benchmark tools ([10-quality](../../docs/plan/10-quality.md) section 2; perf CI v0 of
[09-milestones](../../docs/plan/09-milestones.md) section 8 week 1).

| Path | What it does |
|---|---|
| `src/` (package `ep9_bench`, bin `ep9-bench`) | Runs a command N times. Writes the instructions retired per run as JSON (schema `ep9-bench/v1`) |
| `jetstream/node-host.mjs` | JSC-shell host API on node, so JetStream's `cli.js` runs under node |
| `jetstream/lists.mjs` | Writes `expectations/jetstream-m2.txt` and `jetstream-m7.txt` from the pinned JetStream |
| `jetstream/run.mjs` | Runs JetStream workloads, one process each, under several configs; prints median scores |

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
- Exit status: 0 counts written; 1 error (the command failed, a counted run gave no count, or the JSON was not written); 2 usage error; 3 no counter (the JSON is still written).
- The JSON names the counter. Do not compare counts from different counters.
- A nonzero exit of the command is an error. The command's output is discarded unless `--show-output`.
- `commit` comes from `GITHUB_SHA` or `EP9_COMMIT`.
- `EP9_BENCH_TIME` replaces `/usr/bin/time`. `tests/testdata/time-no-pmc.sh` is a fake that prints what a host with no PMC access prints.
- Tests: `cargo test -p ep9_bench`. The tests probe the OS tool themselves (not through ep9-bench). With a counter, `counts_instructions` checks the counts. Without one, it prints `SKIPPED the count check` and checks the no-counter JSON instead. On macOS, `EP9_BENCH_TIME=$PWD/tools/bench/tests/testdata/time-no-pmc.sh cargo test -p ep9_bench` runs that path.

## JetStream

Needs `bash scripts/sync-bench.sh` first.

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
