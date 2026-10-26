# M0 milestone report (2026-10-26)

- **For:** the owner. [ADR README §2](../adr/README.md#2-status-and-life-cycle) says the milestone report tells the user about each move to Frozen. [§3.3](../adr/README.md#33-change-kinds-and-g8-p-2) says it lists the pre-freeze churn.
- **Exit head:** `86be16b` on `m0/m3` (CI run 40244283271, 46 of 46). The code is the code of `a99b014` (the tag `m3-p4`); M3-P5 changed only `docs/`.
- **Sources:** [decisions log](../decisions-log.md) §13.7 (rows M3-P5-*), [spike review 2](2026-10-26-m0-spike-review.md), [perf run 4](2026-10-26-perf-v0-run4.md), [09-milestones §2.2](../plan/09-milestones.md#22-scope-and-exit-criteria) (the 12 exit items).

## 1. Verdict

M0 exits with one named exception, if the x86-64 Linux sandbox column gives the same result. The pass rule (plan v8 §7.1) has 4 exit columns: mac a64, linux a64, x86-64 Linux in the sandbox, and Windows from the exit-head CI run. Exit items 1-4 and 6-12 are PASS on the 3 columns run so far: mac a64, linux a64 and Windows CI (item 4 on Windows is N/A by design). The x86-64 Linux sandbox column is X86-EXIT-PENDING. Item 5 is OPEN on one cell only: SP-Dispatch condition 2 on AMD EPYC 7763 (Zen 3), sysv and win64. The orchestrator applied the V8-A19 rule before the exit run and got branch 3 (decision M3-P5-V8A19). Branch 3 says: the gate stays, the cell stays OPEN, and M0 exits with this exception carried to M1. On the 3 columns run so far, each `cargo xtask m0-exit` run exits 1 only for that cell. The Windows-as-host extra run also exits 1 for items 7 and 12, by design (§2). This is a statement, not a question: the owner may override it.

```text
 V8-A19 rule for exit item 5 (applied at GATES-2 and again before the exit run)
 (1) non-void PASS on EPYC 7763? ---- no: 4 readings at the M3-P4 code, all void,
                                          branchy 1.928, 1.673, 1.666, 1.724 (limit 1.5)
 (2) gap in the cgoto.c baseline? --- no: 3 cgoto.c forms give 1.66 to 2.17 on EPYC 7763
 (3) else -------------------------> gate stays; item 5 OPEN on one cell; carried to M1
```

## 2. Exit table

The M0 exit run is GATES-3 at `86be16b` (decision M3-P5-EXIT-RUN). It reads the inputs of CI run 40244283271 and perf run 4, with no `--verdicts` file. P = PASS.

| # | Exit item | mac a64 | linux a64 | x86-64 Linux sandbox | Windows CI | Evidence (one line) |
|---|---|---|---|---|---|---|
| 1 | Full test262 run ≤ 10 min | P | P | X86-EXIT-PENDING | P | 93.4 s on macos-arm64, 52.9 s on linux-arm64, 108 s Windows step; 0 new failures, 0 stale lines |
| 2 | Parse negatives all; positives ≥ 99.98% | P | P | X86-EXIT-PENDING | P | negatives 4,653 / 4,653; positives 47,357 / 47,364 (99.985%) |
| 3 | `language/` ≥ 40% | P | P | X86-EXIT-PENDING | P | 22,819 / 23,726 files (96.18%) |
| 4 | richards, deltablue ≥ 0.6x QuickJS | P | P | X86-EXIT-PENDING | N/A | perf run 4 at `a99b014`: 1.617x and 1.557x; no change under `crates/` or `tools/bench/` since. Windows: by design (V8-A22) |
| 5 | Spikes green; SP-tsrs-win recorded | OPEN | OPEN | X86-EXIT-PENDING | OPEN | SP-Dispatch "GO with conditions (condition 2, §3)"; every other spike GO; SP-tsrs-win recorded |
| 6 | GC-stress passes every passing test | P | P | X86-EXIT-PENDING | P | 35,505 / 53,572 files in 6 shards, 0 new failures, 44 skipped; Windows 6 of 6 shard jobs |
| 7 | Nesting caps; UTF-16 column | P | P | X86-EXIT-PENDING | P | the 3 named tests pass; 2 cargo runs, each with exit status 0 |
| 8 | Value, JsResult, word0 are 8 B | P | P | X86-EXIT-PENDING | P | `cargo check -p ep9_value` per target; Windows build step |
| 9 | Every "Fix by M0" row merged; builtin ABI frozen | P | P | X86-EXIT-PENDING | P | 28 of 28 rows due at M0 merged; H20 Frozen |
| 10 | A0 part 1 ADRs merged | P | P | X86-EXIT-PENDING | P | 10 of 10 Accepted |
| 11 | SP-Unit green on 4 targets | P | P | X86-EXIT-PENDING | P | SP-Unit GO in spike review 2 |
| 12 | Identical bytecode hashes | P | P | X86-EXIT-PENDING | P | BCG-CORPUS-14 `17dc1f9c…7531` over 94,502 units, release = debug |

Extra columns, not exit columns: linux x64 from the CI inputs has the same cells. Windows as host has item 7 NOT-RUN and item 12 OPEN, by design (the windows-x64 artifact has no nesting log and no debug corpus log), so that run exits 1 for items 5, 7 and 12. An independent checker ran the 4 m0-exit columns again and got byte-identical reports.

## 3. What M0 built

M0 scope ([09 §2.2](../plan/09-milestones.md#22-scope-and-exit-criteria)): contracts, spikes, the skeleton and A0 part 1. Each month logged its work in the [decisions log](../decisions-log.md) (§2-§11 month 1, §12 month 2, §13 month 3).

```text
 month 1  workspace, CI on 4 targets, Value, cage, heap S0, front end F1, bytecode, VM,
          shapes, ICs, strings, builtin ABI draft, test262 runner, spike review 1
 month 2  classes, with and eval, Promise, Reflect, Spike B (placement (b), gate window 2)
 month 3  P0 ABI 5, OBJECT_LAYOUT 3, x86-64 placement (b)
          P1 realms, async functions, JSON, Date, collections, SP-Unit
          P2 async generators, arguments, ES modules, small builtins, PERF3
          P3 import() and top-level await, facts export, scoped embedder API, H20 freeze inputs
          P4 --facts-out, T0 fast paths, eval E6/E9/D3d, H16 Accepted
          P5 H20 Accepted and Frozen, spike review 2, gate window 4, exit run
```

Also built ahead of the milestone that names it ([09 §8](../plan/09-milestones.md#8-m0-first-month-week-by-week)): generators and async (M1 scope), N realms per isolate (M1), ES modules with `import()` and top-level await (M3), Date (M3), and JSON, Map, Set, WeakMap and WeakSet.

test262 files that pass in month 3 (all files, of 53,616; one `#` per 1,000):

```text
 start (M3-P0)  26,741  ###########################
 M3-P1          29,279  #############################
 M3-P2          34,615  ###################################
 M3-P3          35,323  ###################################
 M3-P4 = exit   35,540  ####################################
```

| Number | Value at the exit | Source |
|---|---|---|
| test262, all files | 35,540 / 53,616; 0 new failures | CI run 40244283271 (GATES-3) |
| test262, `language/` | 22,819 / 23,726 (96.18%) | CI run 40244283271 |
| GC stress | 35,505 / 53,572 (44 skipped), 0 new failures | 6 shards per target |
| Corpus digest | BCG-CORPUS-14 `17dc1f9cc82df11a2db350ff606ed17d8e8811ee1140c126b03d6b4292da7531`, 94,502 units | decisions log BCG-CORPUS-14 |
| Perf run 4, net ratio against QuickJS | richards 1.617 [1.598, 1.630], deltablue 1.557 [1.521, 1.576]; geomean of 6 workloads 1.568 | [perf run 4](2026-10-26-perf-v0-run4.md) |
| Perf run 4, geomean against node --jitless | 1.155 (net) | perf run 4 |
| x86 cachegrind, M3-P4 (`bc63674` against `ec4b841`) | t0-loop -10.10%, richards -12.13%, t0-prop -2.97%, ci-v0 -2.80%, t0-call -2.75%, deltablue -0.26%, t0-branchy 0.00% | decisions log M3-P4-NATIVE-INV |
| ADRs | 9 Draft, 2 Not started, 25 Accepted, 1 Frozen | [ADR README](../adr/README.md) Counts line |

A ratio above 1 means that ep9 is faster. Perf run 4 used the stub harness (decision M3-P2-BASEJS).

## 4. ADR status

**Move to Frozen.** On 2026-10-26 the H20 core (the builtin ABI) went from Accepted to Frozen at the M0 exit (decision M3-P5-H20-FROZEN, commit `b498a01`). It was Accepted earlier the same day (M3-P5-H20-ACCEPT, `4fc6938`). A second reviewer agent approved both steps (M3-P5-H20-APPROVAL, `86be16b`).

- The core: the ABI types, the signature of each public `Cx` method, the table types, the public paths and the Rules rows. The golden file `crates/ep9_vm/tests/golden/h20-core.txt` is a tripwire for them. From now on a change to a core line needs an approved amendment of a Frozen part ([README §3.2](../adr/README.md#32-who-approves-p-1)).
- Part 2 stays append-only: the IntrinsicId, BuiltinId, ObjectClass and `Cx`-method lists take new entries as a fill. An entry that is in keeps its id and signature. G8 does not count an addition.

```text
 Not started  H21 H37                                           (2)
 Draft        H17 H18 H19 H29 H30 H31 H32 H33 H34                (9)
 Accepted     H1-H16, H22-H28, H35, H36                         (25)
 Frozen       H20 (core; part 2 append-only)                     (1)
```

**Pre-freeze churn.** H20's Change log has 25 rows of kind rewrite. All are dated before the freeze (2026-10-21 to 2026-10-25), so G8 does not count them. 16 of them are API additions (new `Cx` methods or ids, no change to an existing signature).

| # | Date | Kind | What changed |
|---|---|---|---|
| 1 | 2026-10-21 | rewrite | R2 list adds `JsStr`, `FlatStr` and boxed closures |
| 2 | 2026-10-21 | rewrite (API addition) | `Cx` index access and iterator protocol |
| 3 | 2026-10-22 | rewrite (API addition) | `Cx` methods for embedders (`run_script`, `global_binding`) |
| 4 | 2026-10-22 | rewrite (API addition) | objects with internal slots; the `ObjectClass` list |
| 5 | 2026-10-22 | rewrite (API addition) | `Cx::evaluate_script` |
| 6 | 2026-10-22 | rewrite (API addition) | the job queue and Promise methods |
| 7 | 2026-10-22 | rewrite (API addition) | `Cx::check_native_stack` |
| 8 | 2026-10-22 | rewrite (API addition) | `get_with_receiver`, `set_with_receiver` |
| 9 | 2026-10-22 | rewrite (API addition) | `IntrinsicId` 44 |
| 10 | 2026-10-22 | rewrite (API addition) | `IntrinsicId` 45-63 declared (M3-P0) |
| 11 | 2026-10-23 | rewrite (pre-freeze churn) | the part-2 row of the receiver methods (CX-ADR2) |
| 12 | 2026-10-23 | rewrite (API addition) | realm methods (`create_realm` and others) |
| 13 | 2026-10-23 | rewrite (API addition) | collection methods |
| 14 | 2026-10-23 | rewrite (API addition) | `Cx::run_module` |
| 15 | 2026-10-23 | rewrite (API addition) | `create_data_property_index_or_throw` |
| 16 | 2026-10-23 | rewrite (API addition) | `Cx::is_error` |
| 17 | 2026-10-25 | rewrite (API addition) | public paths: each `abi` name also at the crate root |
| 18 | 2026-10-25 | rewrite (API addition) | `CollectionCursor` public paths |
| 19 | 2026-10-25 | rewrite | the core golden expands local macros (fix round 2) |
| 20 | 2026-10-25 | rewrite | the core golden reads target-gated `Isolate` methods (fix round 3) |
| 21 | 2026-10-25 | rewrite | H20-Q4 corrected; the embedder core widened (fix round 5) |
| 22 | 2026-10-25 | rewrite | the core golden fails closed (fix round 6) |
| 23 | 2026-10-25 | rewrite | one entry-point rule (fix round 6) |
| 24 | 2026-10-25 | rewrite | the core golden is a tripwire (fix round 7) |
| 25 | 2026-10-25 | rewrite | the `Downcast` kind test needs a sealed token (fix round 9) |

**G8 = 0.** G8 counts only rewrites on a Frozen part dated after M1 exit. No part is in that state yet.

## 5. Open items carried to M1

| Item | Owner | Decision |
|---|---|---|
| Exit item 5: SP-Dispatch condition 2 on AMD EPYC 7763 (Zen 3), sysv and win64 (H15-Q5) | the orchestrator's CI-recorded run | M3-P5-V8A19 |
| PRE-1: in strict code, oxc_semantic rejects `[v = eval("1")] = []` (node gives 1) | M1 front end (check the pinned oxc; report upstream) | M3-P4-OPEN |
| Eval message texts (known differences 1 and 2; one SyntaxError text) | M1, sorted (c) | M3-P4-OPEN |
| PERF3 step-2 items 1-4 (cold construct path, x86 handler pushes, top-level feedback vector, budget phase lock) | M1 | M3-P4-PERF3-S2-M1 |
| A faster GC-stress heap verify; `has-instance-jitted.js` stays on the skip list | M1 | M3-P4-GCSKIP-WIN |
| Should `take_unhandled_rejections` run the jobs its ToString queues? | product call: an M1 embedder-API question for H22 or H20 | M3-P4-UNHANDLED-JOBS-KEEP |
| H20 Open questions Q1, Q3, Q5, Q6 (Q6: a compiler-derived public-API check) | H20, due M1 | M3-P3-OPEN; H20 |
| H20-Q4-X3 type-level fix (a `Local` in Rust heap memory inside one `with_cx` scope) | product call, M1, H10/H20 | M3-P3-H20-X3 |
| M3-P5 notes: A9-3, V9-5, F4, F7, the GATES-3 tool notes; facts for the prototype of `function F(){}` give Unknown (M3-P3 candidate) | M1 planning, sorted (c) | M3-P5-FINDINGS; M3-P3-OPEN |

Still open in W12 (the orchestrator): the x86-64 Linux sandbox column of the exit run (X86-EXIT-PENDING). Done in W12: the SP-Dispatch retry on the exit head. Only linux-x64 ran again; it drew an Intel Xeon Platinum 8573C and passed (not void, branchy 0.968). GitHub did not re-run windows-x64. The pass is not on EPYC 7763, so item 5 stays OPEN (M3-P5-W12-SPD). Closed in M3-P5: RF1-3 (the H23 flag name), H20 row 3 and the Accept and Frozen steps, spike review 2, the CI run of the M3-P4 head, and exit item 4 (M3-P5-OPEN).

## 6. Next

- W12 (plan v8 §7.2): the x86-64 sandbox column. The SP-Dispatch retry is done and gave no EPYC 7763 reading, so branch 3 stands. The orchestrator does no more W12 retries (M3-P5-W12-SPD).
- Then month 4: M1 planning. M1 is the perf vertical slice and A0 part 2 ([09 §2.2](../plan/09-milestones.md#22-scope-and-exit-criteria)). The items in §5 go into that plan.
