# m1-exit test fixtures

Inputs for the unit tests in `tools/xtask/src/m1_exit.rs` (plan v9 rule 24: the tests read only
these files, never the live ADR README or an expectation file). The tests copy them into a
temporary git tree.

| File | Copied to | Used for |
|---|---|---|
| `adr-README.md` | `docs/adr/README.md` | the ADR Counts line (extra row) |
| `m0-spike-review.md` | `docs/measurements/2026-10-26-m0-spike-review.md` | the carried M0 item-5 cell (extra row) |
| `exec.json` | (read in place) | a given `--exec` / `--exec-gc` input |
| `ci-run.json` | (read in place) | a given `--ci-run-json` input |
| `nesting.log` | (read in place) | item 8: a `--nesting` log with the 9 M1 tests ok in 2 cargo runs, and no `m1-exit source:` line (so it is a stale input; the tests add the line with the fake repo's commit) |
| `verdicts.md` | `inputs/` (the tests put the fake repo's HEAD in place of the placeholder commit `c0` x 20) | items 7 and 10: a `--verdicts` table in the V10-D37 form (Spike, Condition, Verdict, Commit, Lowering, Host/CPU, ADRs) with Spike C C1-C4 GO, lowering `jit`, and SP-Facts C5a and C5b GO, C5c "GO, speed recorded", lowering `jit, PROFILED input` (MC-w17-9; one row per condition, M5-P4-ORCH-3) |
| `fuzz-fuzzilli.json`, `fuzz-fuzzilli-dirty.json`, `fuzz-cargo-fuzz.json` | `inputs/` (the tests put the fake repo's HEAD in place of the placeholder commit `f4` x 20) | item 5 (F4): ep9-fuzz/v1 records made by the producer, `fuzz/scripts/record.sh`: a clean counted Fuzzilli record (100 CPU-h), a dirty one of the same campaign id, and a cargo-fuzz record in the fuzz.yml form (`--run` with a run.sh summary, 24 CPU-h). `make-fuzz-records.sh` makes them again |
| `exit2-thp-madvise.json`, `exit2-thp-null.json` | (read in place) | item 2 (MC-w18-O4): ep9-exit2/v1 JSONs made by the producer: `tools/bench/cross/testdata/cross-exit2.json` with the THP mode that `tools/bench/lib/host.mjs` `thpMode` reads from a fake Linux file (`madvise`, or null for no file) in `host.measured`, through `exit2.mjs`. The node test `exit2: host.transparent_hugepage ...` (`tools/bench/cross/run.test.mjs`) checks that they are its output; `EP9_BLESS=1 node --test tools/bench/cross` makes them again |

The tests write the other inputs (kernels, exit2, exec, GC-stress and the other fuzz JSONs, ADR files with
Freeze tables, a small test262 checkout) into the temporary tree at run time, so each test states
the exact input it checks.

Stub-marker files are written by the tests at run time, so this tree holds no marker.
