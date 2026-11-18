# x86 TS time round: the measured numbers (2026-11-19)

- These are the numbers behind [R21](../../21-m2-x86-ts-time.md): the need per x86 TS cell, the Ir-to-time factor, the real counters, the alignment A/B, the TS hot paths, the dead `Ldar` count, the upstream cites and the two lever sums.
- They were copied into the repo for the M10-P0 base (plan v15 pre-step step 9a, DL M10-PLAN-D52). Before the copy they existed only on the research host, in the month-10 plan research and in the reading files of the Path B, GW11b and X86-JITLESS-READ runs.
- `findings.json` was written by a script from the numbers in those files, and each value was checked against its source. A group read from a run has a `source` key. The other groups (the need summary, the Ir change, the penalty, the sums) are derived from those groups; R21 shows the arithmetic.

## Layout of `findings.json`

One object.

| Key | What |
|---|---|
| `need_per_cell` | 8 x86 TS cells (linux-x64 2 boxes, win-x64 runners w3 and w4; jit and jitless): median, interval, `x_median` = 1 / median, `x_low_end` = 1 / low end |
| `need_summary`, `month9_move`, `windows_0966_cross_host_low_end` | the need per host and build, the month-9 move of the 2-box mean, the 09:66 cross-host form (not a gate-host reading) |
| `mac_gw11b` | the Mac a64 TS cells and the GW11b blocks, for comparison |
| `ts_ir`, `ts_ir_change_pct` | the `octane-typescript` bench base rows (cachegrind Ir) at fd9c12ce (the `m8-p5` code) and at 86e39c65 (`m9-p5`), and their change |
| `x86_penalty`, `ir_to_time_factor`, `factor_use` | Mac ratio / x86 ratio per month; the time per 1% Ir (x86 0.31-0.46, Mac a64 0.49-0.69); which range a lever estimate uses |
| `real_counters_ts_jit_m8p5`, `alignment_ab_m8p5`, `jitless_over_jit_ts_m9p5` | perf counters on the Zen 4 box against the cachegrind model; the 64 B handler alignment A/B; the jitless loss at `m9-p5` |
| `ts_hot_paths_m8p5_x64_jitless`, `ts_cold_store_after_m9_x64_jit_pct` | Ir groups, dispatch counts, IC cold rates, operand decode and prefix costs; the cold store path after month 9 |
| `dead_ldar` | the static dead `Ldar` count on TS and the dynamic estimate |
| `upstream_forms` | the V8 lazy accumulator, the V8 Star lookahead, the V8 pinned handler registers, the QuickJS stack machine, each with `owner/repo@rev path:line` cites |
| `star_fuse_m9` | the month-9 STAR-FUSE numbers (a Mac time win, an Ir floor fail, the revert) |
| `sums.research`, `sums.measured_factor` | the plan-research lever sum (stated and recomputed) and the measured-factor sum of plan v15 §0, with the rest for month 11 |
| `prealloc_bestfit`, `plan_v15_rows` | the M10-P0 base merges; the DL rows that took the results |

## Cite forms

- `MR9:<line>` = `docs/measurements/2026-11-19-m2-month9.md`; `DL:<line>` and `DL <row id>` = `docs/decisions-log.md`. Both at the month-end docs head 15f2ba11.
- `XJR` = the X86-JITLESS-READ box summary at `m8-p5` (06486612), summarized in DL M9-P0-D1.
- `bench log <run id> <job id>:<line>` = a GitHub bench-mode log of this repo.
- ep9 paths (`crates/...`, `docs/adr/...`) are at tag `m9-p5` (86e39c65).
- Upstream: `v8/v8@40f4b69c3` and `bellard/quickjs@712e8c0c` (local clones of 2026-11-02). A cite that starts with `src/` or `:` is in the same tree as the cite before it.

## Not here

The raw box outputs (perf stat lines, per-function cachegrind tables, run JSON) stay on the research host. The numbers that R21 uses are all in `findings.json`.
