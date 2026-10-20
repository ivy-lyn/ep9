# Research artifacts

These are the research harnesses that the plan uses again, and the decision inputs that later steps need. All other harnesses stayed in a session scratchpad and were not kept. The reports in `../` describe their results.

| Directory | What it does | Used by |
|---|---|---|
| `tsrs-type-oracle/` | Type-checks a TS project with tsrs, exports types per node, and joins them to oxc spans | Seed of `ep9-typecheck` ([../13-tsrs-type-oracle.md](../13-tsrs-type-oracle.md)) |
| `tsrs-conformance/` | Runs the tsrs parser and oxc on test262 and scores early errors | Re-check after each `third_party/tsrs.commit` bump ([../02-tsrs-conformance.md](../02-tsrs-conformance.md)) |
| `sound-regions/` | Region finder (tsrs-based, `region-facts/` + `finder/`), hot-time mapper for V8 CPU profiles (`hot/`), breaker probes B01-B28c and the interrupt probe (`probes/`), and the results that [../18-sound-regions.md](../18-sound-regions.md) cites (`results/`). See [sound-regions/README.md](sound-regions/README.md) | SP-RegionCoverage (A1h), SP-RegionBound (M7) |
| `decide/` | Inputs of the 2026-10-20 decision round that later steps reuse: CI scope script, `deny.toml` and `clippy.toml` drafts, the upstream README draft, the H11 barrier models and the H36 math tables. See [decide/README.md](decide/README.md) | CI scope job, licence check, fp-check, H11 model test port, H36 ([../../decisions-log.md §5](../../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction)) |

## Build

1. Run `scripts/sync-tsrs.sh` from the repo root.
2. Run `cargo build --release` in each crate directory. Each crate has its own `[workspace]`, so it does not join the ep9 workspace.

Path dependencies point to `third_party/tsrs` relative to the repo root. `harness-oxc` builds the oxc-only variant, which keeps the tsrs Unicode pin out of the oxc build.

## Notes

- The code was written for tsrs `f527e984` and oxc `0.155.0`. tsrs internals change fast, so a newer pin can need small fixes.
- `tsrs-type-oracle/scripts/run.sh` writes to `$OUT` (default: `tsrs-type-oracle/out/`). That directory is git-ignored.
- `tsrs-conformance/results/summary.csv` holds the measured scores from 2026-10-20.
