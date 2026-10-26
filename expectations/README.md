# expectations/

Committed expectations and pinned benchmark lists ([08-repo](../docs/plan/08-repo.md) section 1;
[10-quality](../docs/plan/10-quality.md) sections 1-3). CI reports only differences against them.

| File | What | Written by |
|---|---|---|
| `test262-parse.<target>.txt` | Parse-only test262 results per target: the failing paths. 5 files (the 4 tier-1 targets and `x86_64-apple-darwin`); each lists the same 7 annexB paths. CI checks the 4 tier-1 files; no run reads the `x86_64-apple-darwin` file now (no local x64 run, decision M3-PLAN-D51) | `cargo run --release -p ep9_test262 -- parse --update-expectations` |
| `test262.<target>.<mode>.txt` | Full test262 results per target and mode: the failing paths. `<mode>` = the `--force-mode` value (`jit` only, until A3). One file per tier-1 target; see [test262 exec files](#test262-exec-files) | `cargo run --release -p ep9_test262 -- exec --update-expectations` |
| `test262-gc-skip.txt` | The tests that a GC-stress run neither runs nor counts (`--skip-list`): each runs past the 60 s timeout under GC-stress. Its header says why each one is there; see [GC-stress skip list](#gc-stress-skip-list) | by hand, with a decision row |
| `jetstream-m7.txt` | JetStream 3 M7 list (G4, M7 exit 3) | `node tools/bench/jetstream/lists.mjs` |
| `jetstream-m2.txt` | JetStream 3 M2 list (M2 exit 7) | `node tools/bench/jetstream/lists.mjs` |
| `jitless-set.txt` | JITLESS-SET: Octane-7, v8-v7 kernels, TS workload | by hand |
| `startup-set.txt` | STARTUP-SET | by hand |
| `short-run.txt` | SHORT-RUN | by hand |
| `hint-set.txt` | HINT-SET | by hand |
| `typed-kernels.txt` | TYPED-KERNELS | by hand |
| `bytecode-corpus.digest` | The G1 corpus digest: SHA-256 over the sorted (path under `third_party/`, mode, unit hash) lines of every unit that BytecodeGen compiles from test262 and the 3 bench pins. Every host and target, debug and release, must give it (CI job `test262`). It changes only with an ABI bump or a codegen change, each with a BCG-CORPUS row in `docs/decisions-log.md`. Orchestrator-only after M2-P0. After M3-P4: BCG-CORPUS-14 (`17dc1f9c…7531`; the E9 kind marker slots changed the units of classes that hold a direct eval). Before it: BCG-CORPUS-13 (`ed74e92f…1c2a`, 94,502 units; M3-P3) | `EP9_BLESS_CORPUS=1 cargo test -p ep9_frontend --release --locked --lib bytecodegen::corpus -- --ignored` |
| `fp-check-allow.toml` | Exceptions for `cargo xtask fp-check` (H36): a banned libm symbol allowed in one function, with a reopen condition. More than 5 entries reopens H36. Format in the file header | by hand |
| `adr-trailer-exceptions.txt` | Commits that touch an H-row crate without an `ADR:` trailer, one line `<sha> <ADR ids> <decisions-log row>` (plan v9 rule 26); `cargo xtask adr-lint --trailers-only` reads it from the work tree. Made at M4-P1 for 2 merge commits (decision M4-P1-TRAILER). Format in the file header | the orchestrator, append only |

Benchmark set files (`*-set.txt`, `short-run.txt`, `typed-kernels.txt`): one entry per line.

```
<id>  <status>  <source>
  status  pinned = every input is pinned in this repo today
          TBD    = an input or its pin does not exist yet (the comment above the entry says what)
  source  <pin>:<path>[,<path>...]   <pin> = the checkout of third_party/<pin>.commit; "-" when TBD
```

Lines that start with `#` are comments. The header of each file names its content, comparators and the gates it feeds.

## Who writes the test262 files

- Only the orchestrator writes `test262.*.jit.txt`, `test262-parse.*.txt`, `test262-gc-skip.txt` and `bytecode-corpus.digest` (decision W4-EXPECT-ORCH; plan v8 §1 rule 3). A role that makes tests pass reports the paths; it does not edit these files.
- After each phase merge the orchestrator writes the exec files from full runs with `--update-expectations`. The diff against the phase base may only take paths out (tests that now pass), plus logged false-pass additions (plan v8 §1.2 step 3). The decisions log names the commit and the count.

## test262 exec files

CI (job `test262`) runs `exec` on each tier-1 runner and compares with the file of the host
target. A difference exits 1. An infrastructure error (panic, timeout, worker crash) exits 2.
Both fail the job.

| Target | Failing files after M3-P4 | Source of the last update |
|---|---|---|
| `aarch64-apple-darwin` | 18,076 | the M3-P4 FIXES step 2 expectations commit (M3-P4-EVAL-SCOPE): 217 paths out (D3d, E6 and E9 in direct eval code); the same 217 paths pass in 3 x86-64 sandbox runs (exec, eager feedback, GC-stress), with 0 new failures |
| `x86_64-unknown-linux-gnu` | 18,076 | the same commit; same body |
| `aarch64-unknown-linux-gnu` | 18,076 | the same commit; same body |
| `x86_64-pc-windows-msvc` | 18,076 | the same commit; same body |

- At test262 d53ebc8b the 4 files list the same paths. Only the header line differs. Each file
  has 3 header lines, so 18,079 lines in all.
- The count of failing files (`aarch64-apple-darwin`) at each phase head:

  | Head | Failing files | Head | Failing files |
  |---|---|---|---|
  | M0 week 4 (`dabc38a`) | 43,847 | m2-p5 (`b6109b3`) | 26,875 |
  | m2-p0 (`15a68bf`) | 38,929 | M3-P0 (`2ba3aa4`) | 26,875 |
  | m2-p1 (`382429c`) | 36,782 | M3-P1 (`cbae45d`) | 24,337 |
  | m2-p2 (`5c9034a`) | 33,910 | M3-P2 (`88dacff`) | 19,001 |
  | m2-p3 (`938d2bc`) | 32,654 | M3-P3 (`63f115a`) | 18,293 |
  | m2-p4 (`c39f782`) | 27,325 | M3-P4 (tag `m3-p4`) | 18,076 |

- When CI shows a difference on one target only, that is a platform difference: find the cause
  before you change the file of that target. A failed job uploads `test262-exec-<runner>`
  (the `--json` output; failing tests have `"status":"fail"`).
- After an engine change, write each file on its own target with `--update-expectations`.
  The sets are the same today, so a copy of the new list with the right header is also correct,
  but CI is the check.

## How CI uses the files

Commands are `cargo run --release --locked -p ep9_test262 -- <args>` unless the row says other.
Each exec run compares with `test262.<host-triple>.jit.txt`; the parse run compares with
`test262-parse.<host-triple>.txt`.

| CI job and step | Command (args) | Targets | Result |
|---|---|---|---|
| `test262`: test262 parse | `parse --json test262-parse.json` | 4 | exit 1 on a change |
| `test262`: test262 exec | `exec --json test262-exec.json` (20 s soft timeout per run, the default) | 4 | exit 1 on a change, exit 2 on an infrastructure error |
| `test262`: eager feedback | `exec --feedback-threshold 0 --json test262-exec-eager.json` | 4 | the same file: the result must not depend on the threshold |
| `test262`: corpus digest (release; debug) | `cargo test -p ep9_frontend [--release] --locked --lib bytecodegen::corpus -- --ignored --nocapture` | 4 (debug: not windows-x64) | the digest must equal `bytecode-corpus.digest` |
| `test262`: atom-key gate (debug) | `EP9_ASSERT_ATOM_KEYS=1 target/debug/ep9-test262 exec --timeout 300` | linux-x64 | needs 0 infrastructure errors; exit 1 (a set difference) is only a warning |
| `test262-gc`: GC-stress + eager feedback | `exec --gc-stress 1 --feedback-threshold 0 --timeout 60 --skip-list expectations/test262-gc-skip.txt --json test262-exec-gc.json <prefixes>` | 4, 6 shards each (24 jobs) | `cargo xtask m0-exit --gc-shard k/6` prints the path prefixes of shard k; a run with prefixes compares only the tests it ran |

The `test262` job uploads `m0-exit-inputs-<target>` (the parse, exec and eager JSON, the corpus
and nesting logs), and each `test262-gc` job uploads `m0-exit-inputs-gc-<target>-<k>-of-6`.
`cargo xtask m0-exit` reads them (the M0 exit check, [09-milestones](../docs/plan/09-milestones.md)
§2.2): item 1 recomputes the exec result against the committed exec file (0 new failures, 0
stale lines); item 2 reads the parse JSON, and its Windows cell needs the Windows parse file to
equal the host's; item 6 merges the 6 shard JSONs of a target into one whole-corpus run, needs
`skipped_paths` to be exactly the paths of `test262-gc-skip.txt`, and recomputes the result
against the exec file without the skip list; item 12 reads the corpus logs.

## GC-stress skip list

`test262-gc-skip.txt` lists 44 paths after M3-P4. Under `--gc-stress 1 --timeout 60` (a
collection before every allocation, the heap verifier after each) each one runs past the timeout,
or can on a busy host. A timeout is an infrastructure error (exit 2), so without the list the
GC-stress run could never pass.

Rules:
- `--skip-list` works only with `--gc-stress` and never with `--update-expectations`. A skipped
  test is neither run nor counted, and the expectation compare ignores it.
- A path goes on the list only with a decision row that gives its time alone under GC-stress
  and its result without GC-stress. The header of the file repeats the reason.
- 9 of the 44 also fail without GC-stress (they are in every exec file): S7.4_A5.js, S7.4_A6.js,
  the 5 `tco-*` tests, `sm/Date/two-digit-years.js` and `sm/Proxy/ownkeys-linear.js`. The other
  35 pass without GC-stress; m0-exit item 6 names them in its evidence and they do not fail the
  item (M2-P2-M0EXIT-GCSKIP).
- A fix that makes a test fast enough under GC-stress takes its path off the list again, with a
  row.

The decision rows, in order:

| Row | Paths added | List size | Why |
|---|---|---|---|
| W4-T262-GC-SKIP | `tco-call-args.js`, `tco-non-eval-function.js`, `tco-non-eval-global.js`, `sm/String/string-upper-lower-mapping.js` | 4 | known GC-stress timeouts in M0 week 4 |
| M2-P1-GEN-GCSKIP | `sm/extensions/recursion.js` | 5 | deep generator-resume recursion; every forced collection scans the deep stack |
| M2-P2-FIX-GCSKIP | `tco-non-eval-with.js`, `sm/Proxy/ownkeys-linear.js` | 7 | more than 400 s alone under GC-stress (tco), 68 s alone at load 9 (ownkeys) |
| M2-P3-GCSKIP | S7.4_A5.js, S7.4_A6.js, `tco-non-eval-function-dynamic.js` | 10 | 289 s, 152 s and 617 s alone under GC-stress |
| M2-P4-GCSKIP-TOSPLICED | `sm/Array/toSpliced-dense.js` | 11 | 692,942 collections per run; 62 s per run alone |
| M3-P1-FIXDOCS-GCSKIP | 8 `sm/Date/dst-offset-caching-*-of-8.js`, `sm/Date/two-digit-years.js`, `sm/regress/regress-1507322-deep-weakmap.js` | 21 | Date and WeakMap tests that time out alone under GC-stress |
| M3-P2-GCSKIP | 21 URI tests, then `decodeURIComponent/S15.1.3.2_A2.4_T1.js` (fix round 2) | 43 | loops over up to 65,536 code units or 5 x 64^3 byte sequences |
| M3-P3-GCSKIP-WIN | `sm/Function/has-instance-jitted.js` | 44 | both runs past 60 s on the windows-x64 GC shard 2/6; 19.3 s alone on mac a64. Why Windows is more than 6x slower is an M3-P4 FIXES item |
| M3-P4-GCSKIP-WIN | none; `has-instance-jitted.js` stays | 44 | the M3-P4 FIXES check: the test is CPU-bound (132,107 full collections, each with a heap verify; 214 G instructions, 10.3 s on mac a64; 45 page commits, 0 decommits). No cost that only Windows has was found; on the GC shards the windows-x64 runner takes about 1.4x the time of Linux and 2x the time of macOS. A faster GC-stress heap verify is an M1 item |

## Change log

| Date | Change |
|---|---|
| 2026-10-21 | `fp-check-allow.toml` with entry 1: `fmod` and `pow` in `oxc_semantic::ts_enum::eval::evaluate_expression` on all targets (TS enum folding; reopen at H3). It is the only finding of `cargo xtask fp-check` on the 5 targets and on x86_64 Linux with `x86-64-v3` (M0 week 4) |
| 2026-10-21 | test262 exec files for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` and `x86_64-pc-windows-msvc` from CI run 39501063919; CI checks all 4 targets |
| 2026-10-21 | 2 tests pass after PR #1 review fixes Fr4 and Fr5 (`super[k] = v` key order; `(a?.b)()` keeps `this`): 43,849 → 43,847 failing files on all 4 targets (commits `1ee8583`, `d514b6e`) |
| 2026-10-21 | `bytecode-corpus.digest` row. M2-P0 re-blessed the digest after ABI 3: `ffdbd454…82acb` → `8203f695…6805b` (macOS arm64, release and debug) |
| 2026-10-25 | Text update for month 3 (M3-P4 DOCS; no expectation file changed): the GC skip-list row and section with its decision rows, who writes the files, the failing-file count per phase head, how CI and `cargo xtask m0-exit` use the files, the 5 parse files and the current corpus digest |
| 2026-10-26 | Text update after M3-P4 (M3-P4 DOCS step 2; no expectation file changed by this row): 18,076 failing files on all 4 targets (217 paths out), the M3-P4 row of the history, BCG-CORPUS-14, and the M3-P4-GCSKIP-WIN row (the skip list stays at 44) |
