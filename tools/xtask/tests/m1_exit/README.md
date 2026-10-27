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
| `nesting.log` | (read in place) | item 8: a `--nesting` log with the 9 M1 tests ok in 2 cargo runs |
| `verdicts.md` | (read in place) | items 7 and 10: a `--verdicts` table with Spike C GO and SP-Facts "GO, speed recorded" |

The tests write the other inputs (kernels, exit2, exec, GC-stress and fuzz JSONs, ADR files with
Freeze tables, a small test262 checkout) into the temporary tree at run time, so each test states
the exact input it checks.

Stub-marker files are written by the tests at run time, so this tree holds no marker.
