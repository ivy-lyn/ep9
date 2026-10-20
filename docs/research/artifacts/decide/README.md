# Decision-round inputs (2026-10-20)

- These files are inputs of the decision round of 2026-10-20 ([decisions-log §5](../../../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction)). Later steps reuse them.
- They were copied from the session scratchpad on 2026-10-21. Before the copy they existed only there. The research path `work/decide/<agent>/...` maps to `decide/<agent>/...` here (table below).
- The files are byte copies, with two exceptions: `governance/scope-tests/expected.txt` is new (the re-run output of 2026-10-21), and the one relative link in `governance/upstream-README.md` now points to `docs/plan/12-governance.md` (it pointed to `../governance.md`, which does not exist).
- Drafts keep their old references. `docs/governance.md` in a draft means [docs/plan/12-governance.md](../../../plan/12-governance.md). `.github/scripts/ci-scope.sh` is the planned home of `governance/ci-scope.sh`.

## Files

| Here | Research path (`work/decide/`) | What it is | Used by |
|---|---|---|---|
| `governance/ci-scope.sh` | `governance/proposed/.github/scripts/ci-scope.sh` | CI scope script: picks the jobs and targets of one CI run (REMOTE-CI) | CI scope job and nightly run (code follow-up 10). The CI owner copies it to `.github/scripts/` |
| `governance/scope-tests/*.txt` | `governance/scope-tests/` | Changed-file lists for the script (`CHANGED_FILES_FILE`) | Same |
| `governance/scope-tests/expected.txt` | (new) | Script output for each list, `pull_request` event, standard cost mode | Same |
| `governance/deny.toml` | `governance/proposed/deny.toml` | Draft of the root `deny.toml` (LICENSE decision) | The root [`deny.toml`](../../../../deny.toml) starts from it |
| `governance/upstream-README.md` | `governance/proposed/docs/upstream/README.md` | Draft index and template for upstream drafts (UPSTREAM decision) | [docs/upstream/README.md](../../../upstream/README.md) starts from it |
| `math/clippy-check/` | `math/exp/cl/` | Draft root `clippy.toml` (H36-lint) and a 3-function crate that checks it | Root `clippy.toml` (code follow-up 5) |
| `math/evidence_tables.txt` | `math/exp/evidence_tables.txt` | H36 bit-equality tables: mismatch share and max ULP per function, per implementation pair (`compare.py` output) | [H36](../../../adr/H36-fp-semantics.md) evidence [M] |
| `math/evidence_bench_a64.txt` | `math/exp/evidence_bench_a64.txt` | aarch64 timing per function. Columns: function, implementation (`std`, `libm`, `pxfm`), ns per call, and a sink flag (`acc.is_nan()`, only there to keep the loop alive; not a result) | H36 evidence |
| `gc-barrier/model/model.py` | `gc-barrier/model/model.py` | Exhaustive interleaving model of one mutator and one concurrent marker: decided barrier form vs 8 other forms, 5 scenarios | [H11](../../../adr/H11-barrier-and-atomics.md) evidence [M]; port to the `ep9_heap` model test (M0 wk2) |
| `gc-barrier/model/model-output.txt` | `gc-barrier/model/model-output.txt` | Its output | Same |
| `critic/los_card_model.py` | `critic/los_card_model.py` | Critic model for large-object cards in the S2 slow path: the first decided text loses an object; a card rescan or a push fixes it | H11 evidence [M]; the `los` scenario of the model test |
| `critic/los_card_model.out` | `critic/los_card_model.out` | Its output | Same |

## Re-run on 2026-10-21 (from the copies here)

| Check | Command (from this folder) | Result |
|---|---|---|
| Barrier model | `python3 -I gc-barrier/model/model.py \| diff - gc-barrier/model/model-output.txt` | No difference. `rw_slowstore` (decided) is ok in all 5 scenarios; `rw_precheck`, `rw_norescan`, `rw_oldfilter`, `rw_elide_escaped`, `satb_new` and `satb_noweakrb` each lose an object |
| LOS card model | `python3 -I critic/los_card_model.py \| diff - critic/los_card_model.out` | No difference: `decided` LOST C; `fix_rescan` ok; `fix_push` ok |
| CI scope script | `shellcheck governance/ci-scope.sh` | Clean |
| CI scope tests | `EVENT=pull_request DRAFT=false FULL=false COST_MODE=standard CHANGED_FILES_FILE=governance/scope-tests/<name>.txt GITHUB_OUTPUT=/dev/null bash governance/ci-scope.sh` | Output equals `expected.txt`. Docs only: no targets. `Cargo.lock` or `crates/ep9_platform`: all 4 targets. `ep9_platform` also turns on the spikes; `third_party/tsrs.commit` turns on the tsrs job |
| clippy.toml draft | `cargo +1.99.1 clippy` in `math/clippy-check/` | 6 `disallowed_methods` warnings, one for each banned call in `src/lib.rs` (`sin`, `exp_m1`, `ln_1p`, `sin_cos`, `powi`, `algebraic_mul`). The exact operations in function `c` give no warning |

## Notes

- `math/clippy-check/` has its own empty `[workspace]`, so it does not join the ep9 workspace. Its `target/` is git-ignored.
- The math tables come from a harness (`math/exp/mathcmp`, input corpus, d8 and node runs) that stayed in the scratchpad. Only the result tables are kept.
- The governance drafts are kept as they were decided. The root `deny.toml` and `docs/upstream/README.md` were edited after the copy (new references, a `[bans]` table, the three drafts).
