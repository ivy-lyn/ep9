# expectations/

Committed expectations and pinned benchmark lists ([08-repo](../docs/plan/08-repo.md) section 1;
[10-quality](../docs/plan/10-quality.md) sections 1-3). CI reports only differences against them.

| File | What | Written by |
|---|---|---|
| `test262-parse.<target>.txt` | Parse-only test262 results per target | `cargo run --release -p ep9_test262 -- parse` |
| `test262.<target>.<mode>.txt` | Full test262 results per target and mode: the failing paths. `<mode>` = the `--force-mode` value (`jit` only, until A3). One file per tier-1 target; see [test262 exec files](#test262-exec-files) | `cargo run --release -p ep9_test262 -- exec --update-expectations` |
| `jetstream-m7.txt` | JetStream 3 M7 list (G4, M7 exit 3) | `node tools/bench/jetstream/lists.mjs` |
| `jetstream-m2.txt` | JetStream 3 M2 list (M2 exit 7) | `node tools/bench/jetstream/lists.mjs` |
| `jitless-set.txt` | JITLESS-SET: Octane-7, v8-v7 kernels, TS workload | by hand |
| `startup-set.txt` | STARTUP-SET | by hand |
| `short-run.txt` | SHORT-RUN | by hand |
| `hint-set.txt` | HINT-SET | by hand |
| `typed-kernels.txt` | TYPED-KERNELS | by hand |

Benchmark set files (`*-set.txt`, `short-run.txt`, `typed-kernels.txt`): one entry per line.

```
<id>  <status>  <source>
  status  pinned = every input is pinned in this repo today
          TBD    = an input or its pin does not exist yet (the comment above the entry says what)
  source  <pin>:<path>[,<path>...]   <pin> = the checkout of third_party/<pin>.commit; "-" when TBD
```

Lines that start with `#` are comments. The header of each file names its content, comparators and the gates it feeds.

## test262 exec files

CI (job `test262`) runs `exec` on each tier-1 runner and compares with the file of the host
target. A difference exits 1. An infrastructure error (panic, timeout, worker crash) exits 2.
Both fail the job.

| Target | Failing files | Source |
|---|---|---|
| `aarch64-apple-darwin` | 43,849 | Local run (M0 week 3). The macos-15 runner writes the same bytes (CI run 39501063919) |
| `x86_64-unknown-linux-gnu` | 43,849 | ubuntu-24.04 runner, CI run 39501063919. Docker on Rosetta writes the same bytes |
| `aarch64-unknown-linux-gnu` | 43,849 | ubuntu-24.04-arm runner, CI run 39501063919. Docker (arm64) writes the same bytes |
| `x86_64-pc-windows-msvc` | 43,849 | windows-2025 runner, CI run 39501063919 |

- At test262 d53ebc8b the 4 files list the same paths. Only the header line differs. The
  `--json` output of the 4 runners also gives the same cause and detail for every failing test.
- When CI shows a difference on one target only, that is a platform difference: find the cause
  before you change the file of that target. A failed job uploads `test262-exec-<runner>`
  (the `--json` output; failing tests have `"status":"fail"`).
- After an engine change, write each file on its own target with `--update-expectations`.
  The sets are the same today, so a copy of the new list with the right header is also correct,
  but CI is the check.

## Change log

| Date | Change |
|---|---|
| 2026-10-21 | test262 exec files for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` and `x86_64-pc-windows-msvc` from CI run 39501063919; CI checks all 4 targets |
