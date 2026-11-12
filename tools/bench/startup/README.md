# tools/bench/startup

STARTUP-LAYOUT (plan v13 section 6.2; route STARTUP-LAYOUT): a linker order file for the start-up
path of `ep9`, and the start-up budget row of perf CI v0.

| File | What it is |
|---|---|
| `order.mjs` | The generator. Runs a coverage build of `ep9` once with `-e 0`, merges the profile with llvm-profdata, writes one symbol per line: every function that ran, most calls first |
| `aarch64-apple-darwin.order` | The order file for the macOS arm64 release build (ld64 `-order_file`), from `order.mjs` at 2b07f2dd |
| `startup.test.mjs` | `node --test tools/bench/startup/startup.test.mjs`: the generator's parts, the order file's form, and the budget row of `.github/scripts/perf-ci-v0.mjs` |

## Make the order file

```bash
CARGO_TARGET_DIR=target/cov RUSTFLAGS="-C instrument-coverage" \
  cargo build --release --locked -p ep9_cli
node tools/bench/startup/order.mjs --cov-bin target/cov/release/ep9 \
  --target aarch64-apple-darwin --out tools/bench/startup/aarch64-apple-darwin.order
```

- The symbol names are v0 names (and the export names of the handlers). They do not depend on
  RUSTFLAGS, so the coverage build gives the names of the release build. They depend on the
  target, so each target needs its own file, made on that target (Linux x86-64: lld
  `--symbol-ordering-file`, the default linker of `x86_64-unknown-linux-gnu`; no file yet).
- The linker skips a line with no symbol: a function that the release build inlines, or one
  that was renamed. Make the file again after a large start-up change.
- `ep9 -e 0` ran 1,499 of 36,100 profiled functions at 2b07f2dd. 181 of them are symbols of the
  release binary (fat LTO inlines the rest); they are on 87 distinct 16 KB text pages, and fit
  in 22.

## The reading at 2b07f2dd (macOS arm64, a loaded host: a report, not a gate reading)

The release `ep9` relinked with `-C link-arg=-Wl,-order_file,<this file>`, against the same build
without it; `/usr/bin/time -l ep9 -e 0`, `EP9_HASH_SEED=1`, 201 paired rounds in alternating
order, load 5-6:

| Metric (median) | Without | With | Change |
|---|---:|---:|---:|
| page reclaims | 547 | 515 | -5.9% |
| maximum resident set size | 5,193,728 B | 4,603,904 B | -11.4% |
| instructions retired | 24,439,664 | 24,237,629 | -0.8% |
| wall time | 4.74 ms | 4.67 ms | paired ratio 0.986, 95% interval [0.974, 1.001] |

The page and memory changes are clear. The wall-time gain is about 1.4%, and its interval holds
1.0: below the noise of this host. So the build does not use the file yet
(`crates/ep9_cli/build.rs` passes no link argument): the plan's rule is "if the gain is below
noise, report it". A quiet-window reading on the gate host decides; the decision is a
decisions-log row. If the build takes the file, `crates/ep9_cli/build.rs` passes
`cargo::rustc-link-arg-bins=-Wl,-order_file,<this file>` for the release profile on
`aarch64-apple-darwin` only, and reruns when the file changes.

## The start-up budget row

`.github/scripts/perf-ci-v0.mjs` counts `ep9 -e 0` as the row `startup` (a default workload)
and reads it against `STARTUP_BUDGETS` (per ep9-bench counter and target): `within`, `OVER
BUDGET` or `no budget for this counter and target`. It is a record: the exit status does not
change; on GitHub an over-budget row is a warning. The CI budget (cachegrind, Linux x86-64) is an
estimate until the first CI reading sets it by a decisions-log row.
