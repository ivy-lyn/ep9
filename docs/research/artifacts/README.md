# Research artifacts

These are the two research harnesses that the plan uses again. All other harnesses stayed in a session scratchpad and were not kept. The reports in `../` describe their results.

| Directory | What it does | Used by |
|---|---|---|
| `tsrs-type-oracle/` | Type-checks a TS project with tsrs, exports types per node, and joins them to oxc spans | Seed of `ep9-typecheck` ([../13-tsrs-type-oracle.md](../13-tsrs-type-oracle.md)) |
| `tsrs-conformance/` | Runs the tsrs parser and oxc on test262 and scores early errors | Re-check after each `third_party/tsrs.commit` bump ([../02-tsrs-conformance.md](../02-tsrs-conformance.md)) |

## Build

1. Run `scripts/sync-tsrs.sh` from the repo root.
2. Run `cargo build --release` in each crate directory. Each crate has its own `[workspace]`, so it does not join the ep9 workspace.

Path dependencies point to `third_party/tsrs` relative to the repo root. `harness-oxc` builds the oxc-only variant, which keeps the tsrs Unicode pin out of the oxc build.

## Notes

- The code was written for tsrs `f527e984` and oxc `0.155.0`. tsrs internals change fast, so a newer pin can need small fixes.
- `tsrs-type-oracle/scripts/run.sh` writes to `$OUT` (default: `tsrs-type-oracle/out/`). That directory is git-ignored.
- `tsrs-conformance/results/summary.csv` holds the measured scores from 2026-10-20.
