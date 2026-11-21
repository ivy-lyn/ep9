# Research artifacts

These are the research harnesses that the plan uses again, the decision inputs that later steps need, and the raw evidence of three rounds (`vm-lessons/`, `x86-ts-time/`, `m2-stuck/`). All other harnesses stayed in a session scratchpad and were not kept. The reports in `../` describe their results.

| Directory | What it does | Used by |
|---|---|---|
| `tsrs-type-oracle/` | Type-checks a TS project with tsrs, exports types per node, and joins them to oxc spans | Seed of `ep9-typecheck` ([../13-tsrs-type-oracle.md](../13-tsrs-type-oracle.md)) |
| `tsrs-conformance/` | Runs the tsrs parser and oxc on test262 and scores early errors | Re-check after each `third_party/tsrs.commit` bump ([../02-tsrs-conformance.md](../02-tsrs-conformance.md)) |
| `sound-regions/` | Region finder (tsrs-based, `region-facts/` + `finder/`), hot-time mapper for V8 CPU profiles (`hot/`), breaker probes B01-B28c and the interrupt probe (`probes/`), and the results that [../18-sound-regions.md](../18-sound-regions.md) cites (`results/`). See [sound-regions/README.md](sound-regions/README.md) | SP-RegionCoverage (A1h), SP-RegionBound (M7) |
| `decide/` | Inputs of the 2026-10-20 decision round that later steps reuse: CI scope script, `deny.toml` and `clippy.toml` drafts, the upstream README draft, the H11 barrier models and the H36 math tables. See [decide/README.md](decide/README.md) | CI scope job, licence check, fp-check, H11 model test port, H36 ([../../decisions-log.md §5](../../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction)) |
| `vm-lessons/` | Raw findings, per-system lessons and verify verdicts of the 2026-11-18 VM-lessons round (.NET, Kotlin, Dart; `findings.json`). See [vm-lessons/README.md](vm-lessons/README.md) | [../19-vm-aot-jit-lessons.md](../19-vm-aot-jit-lessons.md) Appendix A; the LRN-* follow-ups of R19 |
| `x86-ts-time/` | The measured numbers of the 2026-11-19 x86 TS time round: the need per x86 TS cell, the Ir-to-time factor, real counters, the alignment A/B, the dead `Ldar` count, upstream cites and the lever sums (`findings.json`), and the month-10 reads (`month10.json`, 2026-11-21). See [x86-ts-time/README.md](x86-ts-time/README.md) | [../21-m2-x86-ts-time.md](../21-m2-x86-ts-time.md); plan v15 (DL M10-PLAN-D52) |
| `m2-stuck/` | The numbers of the 2026-11-21 "why M2 is stuck" round: the ranked causes and verdicts, the cycle buckets against node, the box-neutral TS series, the month-10 lever ledger, the per-host TS cells, the gate and process costs, the orchestrator decisions D1-D7 and the X1 and X2 experiment numbers (`findings.json`). See [m2-stuck/README.md](m2-stuck/README.md) | [../22-m2-stuck-causes.md](../22-m2-stuck-causes.md); plan v16 |

## Build

1. Run `scripts/sync-tsrs.sh` from the repo root.
2. Run `cargo build --release` in each crate directory. Each crate has its own `[workspace]`, so it does not join the ep9 workspace.

Path dependencies point to `third_party/tsrs` relative to the repo root. `harness-oxc` builds the oxc-only variant, which keeps the tsrs Unicode pin out of the oxc build.

## Notes

- The code was written for tsrs `f527e984` and oxc `0.155.0`. tsrs internals change fast, so a newer pin can need small fixes.
- `tsrs-type-oracle/scripts/run.sh` writes to `$OUT` (default: `tsrs-type-oracle/out/`). That directory is git-ignored.
- `tsrs-conformance/results/summary.csv` holds the measured scores from 2026-10-20.
