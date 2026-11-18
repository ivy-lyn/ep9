# P-gates P1, P2 and P4: the exit form (2026-11-16, M9-P1 EXIT-INPUTS)

This file is the M2 exit item 9 input in the form that `cargo xtask m2-exit --p-gates` reads: the first Markdown table with the columns `Gate` and `Result`, one row each for P1, P2 and P4, and the result `not tripped` or `tripped` (plan v14 §4.1 (3); route EXIT-INPUTS). The report of 2026-11-09 ([2026-11-09-p-gates.md](2026-11-09-p-gates.md)) has a `Trips now?` column, which the reader does not take. The P-gates of [01-front-end §7](../plan/01-front-end.md#7-oxc-upstream-track-local-patches-p-gates-unchanged) start option D (an own parser) when one trips.

- **Tree:** `142e1b7a` (the `m9-p0` tag commit; oxc `=0.158.0` in the root `Cargo.toml`). **test262:** the `m0-exit-inputs` artifacts of CI 43822657071 on `9b53cf14` (`142e1b7a` adds docs only), pin `d53ebc8b`.
- **Who reads it again:** the month-end rehearsal and the exit reading. Each DEPS-BUMP run adds a row to the P2 table below (plan v14 §9.3; IN14 #148), and the reading changes when a bump needs a local oxc patch or a P4 file fails.

## Exit form

| Gate | Result | Trigger (01 §7) | Reading at `142e1b7a` | Evidence |
|---|---|---|---|---|
| P1 | not tripped | Eval context or the depth limit is neither upstream nor in a local set of 3 or fewer patches | Both stay on their tested ep9 workarounds: the eval context (`crates/ep9_frontend/src/eval.rs`) and the depth layers (`crates/ep9_frontend/src/depth.rs`). oxc 0.158.0 `ParseOptions` has the same fields as 0.157.0 (no eval-context option, no depth limit). Local oxc patches: 0 of 3 (`third_party/patches/` holds fuzzilli, icu4x and regress only; no oxc line in `[patch.crates-io]`) | 2026-11-09-p-gates.md (0.157.0); DEPS-BUMP-1 to 0.158.0 with no oxc patch (orchestrator log M6-ORCH-396); the oxc_parser 0.157.0 and 0.158.0 sources |
| P2 | not tripped | Measured work for oxc version bumps is more than 1 PM per year (est.) | 2 measured bumps: 0.155.0 to 0.157.0 took 0.59 agent-hours; DEPS-BUMP-1 (0.157.0 to 0.158.0 and the lock-only moves) took 1.73 hours of workflow wall time. At one bump each 14 days (26 a year), the larger reading gives about 45 hours a year (est.), below 1 PM (about 160 hours) | the P2 table below |
| P4 | not tripped | Any test262 lone-surrogate eval case fails through the shim | The 11 files of the P4 baseline (`probe::p4_lone_surrogate_eval_count`: 3 literal, 8 generated) and the 4 files of its "review by hand" list are not in the failed lists of the 8 runs (4 targets, exec and exec eager; 52,892 of 53,616 files pass in each, 0 skipped) | CI 43822657071, `m0-exit-inputs-<target>/test262-exec.json` and `test262-exec-eager.json` |

## P2: hours per oxc bump

| Run | Date | Move | Hours | Source |
|---|---|---|---|---|
| M7-P4 OXCBUMP | 2026-11-09 | oxc 0.155.0 to 0.157.0 | 0.59 agent-hours (one agent; 35.5 min wall) | 2026-11-09-p-gates.md, "P2: the bump on a clock" |
| DEPS-BUMP-1 | 2026-11-10 to 2026-11-11 | oxc 0.157.0 to 0.158.0, plus lock-only moves (loom, generator, itoa, smallvec, syn, unicode-width, cc, memmap2) | 1.73 hours of workflow wall time (23:18 to 01:02, +08:00; 7 agents: scout, builder, checker r1-r3 with 2 fix rounds) | orchestrator log M6-ORCH-385 (start) and M6-ORCH-396 (done) |
| DEPS-BUMP-2 | due 2026-11-24 | - | not run yet | plan v14 §9.3 |

**Limits.** The DEPS-BUMP-1 hours are the wall time of a workflow, not the sum of the agent hours, and they include the other crates of the bump. Two bumps are a small sample; a release that changes the AST or the `Semantic` API costs more (2026-11-09-p-gates.md). The reading is not a gate host reading and needs none (item 9 is a check of the gates, not a speed cell).
