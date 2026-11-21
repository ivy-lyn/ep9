# Why M2 is stuck: the numbers (2026-11-21)

- These are the numbers behind [R22](../../22-m2-stuck-causes.md): the ranked causes C1-C6 and their verdicts, the cycle buckets against node, the box-neutral TS series, the month-10 lever ledger, the per-host TS cells at `m10-p5`, the gate cells, the process costs, the orchestrator decisions D1-D7, and the numbers of the experiments X1 (bytes against placement) and X2 (dynamic counts).
- `findings.json` was written by hand from the research report, the two experiment reports and the experiment verifier. Each group has a `source` key. The sources live in the orchestrator scratchpad; R22 names them by short form.

## Layout of `findings.json`

One object, schema `ep9-research/m2-stuck/v1`.

| Key | What |
|---|---|
| `tree`, `short_forms` | the trees (`m10-p5`, mid 2856a698, `m9-p5`) and the cite and ratio forms |
| `need` | the item 3 low ends and the need per build at `m10-p5` |
| `ranked_causes` | C1-C6: kind, share of the need, claim, evidence numbers, verdict, source |
| `not_causes` | a missing JIT, the workload and harness, the build flags |
| `cycle_buckets_jitless_vs_node_m9p5_G` | the per-bucket cycles of ep9 jitless and node --jitless (1 box, `m9-p5`) |
| `box_neutral_series` | ep9 / QuickJS per box and 2-box mean from the month-7 end to the month-10 end, node / QuickJS, same-round t(jitless)/t(jit), x64 TS Ir, the month moves and the time per 1% Ir |
| `lever_ledger_m10` | one row per month-10 lever: Ir change, x86 time read, merged or not, what decided, DL row; and the rate per cell |
| `hosts_m10p5_ts` | TS against node --jitless on every x86 host read at `m10-p5`, and the LEVER-13 shift per host |
| `gate_cells_m10p5` | the ENG and PROC cell counts, the item 8 arithmetic, the Mac window count |
| `process_costs_m10` | hours, tags, runs and tokens per month; the month-10 token, wall and box-h splits |
| `x1_bytes_vs_placement` | the X1 compare counts, the LEVER-13 byte change, the placement moves, the box offset fit and the verifier corrections |
| `x2_dynamic_counts` | the X2 totals, the Ldar classes, the prefixed ops by cause, the two lever verdicts and their est. savings |
| `decisions_orchestrator` | D1-D7 of the orchestrator log row M6-ORCH-1261: decisions, not findings |
| `plan_v16_input_from_xv` | the first-box order that the experiment verifier proposed (a plan input) |

## Cite forms

- `PBe:<line>` = the month-end Path B compare file; `TD` = the X86-TOPDOWN-READ summary (`m9-p5` code, 1 box; DL M10-P0-WPRE-1).
- `VE`, `VL`, `XV` = the engine verifier, the ledger / gate / process verifier and the experiment verifier; `X1`, `X2` = the experiment reports; `A`..`E` = the angle reports.
- `OL M6-ORCH-<n>` = a row of the orchestrator log. `DL <row id>` = a row of `docs/decisions-log.md`. `P15:<line>` = plan v15.
- ep9 paths (`crates/...`, `docs/...`) are at tag `m10-p5` (591064fc).

## Not here

The run JSON, the perf files, the objdump listings (about 61,000 lines per build), the nm tables, the counter patch, the raw count files and the static bytecode dump stay in the orchestrator scratchpad. The cross-build binaries were deleted after X1. No fuzz input is in this directory.
