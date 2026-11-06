# M1 exit report (2026-11-05)

- **For:** the owner. [ADR README §2](../adr/README.md#2-status-and-life-cycle) says the milestone report tells the user about each move to Frozen. [§3.3](../adr/README.md#33-change-kinds-and-g8-p-2) says it lists the Frozen-part rewrites between M1a and M1 exit by name.
- **Milestone:** M1 = the perf vertical slice and A0 part 2 ([09 §2.2](../plan/09-milestones.md#22-scope-and-exit-criteria), the 10 M1 exit items). M1a (H17, H18, H19 cores Frozen) was the month-5 step ([M1a report](2026-11-02-m1a-report.md)).
- **Head:** the columns ran at X = `c5588318` on `m1m6/p5` (decisions log M6-P5-FUZZ-TOPUP). The report commits after X (this file, then its M1-CHECK fixes) change only `docs/measurements` (this file). The code is the code of CF `ef0c1ae6` (the tag `m6-p3`). Since CF only `docs/**` changed, plus one line in `tools/xtask/src/exit/adrline-pins.tsv` (the H19 pin, decisions log M6-P4-FROZEN).
- **CI:** run 41864288129 on CF (event `pull_request`, attempt 5: 62 of 62 jobs `success`; decisions log §16.6). Every CI input of this report is an artifact of that run.
- **Sources:** [decisions log](../decisions-log.md) §16.5 to §16.7; [perf run 8](2026-11-05-perf-v0-run8.md); [x86 run 8](2026-11-05-perf-v0-run8-x86.md); [Spike C Verdict rows](2026-11-05-spike-c.md); the ADR files at HEAD.
- **Rule for numbers:** each number names the file it comes from. Files that are not in the repo are "scratchpad" files of the orchestrator. A number that this report could not check is marked **(not verified)**.

```text
 CF ef0c1ae6 (tag m6-p3) ──> CI run 41864288129 (62/62)
        │
        ├─ gate window 8 (Mac, gate host) + x86 run 8 (remote box, non-gate)
        ├─ counted fuzz: cargo-fuzz run 2 (25.12 CPU-h); Fuzzilli wave 1 (56.0 CPU-h)
        ├─ M6-P4  B2 text; the one Frozen step aa3807aa; records; APPROVER-M6 8e50ed4f  = tag m6-p4 (1637b798)
        ├─ M6-P5  Frozen-part notes (H16, H17, H32); records; APPROVER-M6 ae834773
        ├─ Fuzzilli top-up cf8-fz-9, cf8-fz-10 (44.67 CPU-h); DL row c5588318 = X
        └─ M1-EXIT: m1-exit x 4 columns at X (this file); report commits after X (report, M1-CHECK fixes)
```

## 1. Result

**Rule (plan v11 V11-D33).** M1 exit holds when all 40 cells (10 items x 4 columns) are PASS or N/A, except the one named cell "item 2, linux x86-64, OPEN marginal (no gate host)" (V9-D31). There is no tool waiver and no other named exception (V11-A11).

**Result (the final run, 2026-11-05 UTC): M1 exit holds.** 39 cells are PASS or N/A. The one other cell is the named cell (item 2, linux x86-64, OPEN), as the rule allows. Item 5 is PASS in all 4 columns: Fuzzilli has 100.67 of 100 counted CPU-h after the month-end top-up (cf8-fz-9 and cf8-fz-10, 44.67 CPU-h; section 4). The 4 columns ran at X = `c5588318` (the tree that the m1-exit outputs name: "tree at commit `c5588318…`"). The report commits after X change only `docs/measurements`.

How it ran:

- Tool: `cargo xtask m1-exit` (tools/xtask/src/m1_exit.rs), built from the HEAD tree, run on mac a64 with `--root` at the HEAD worktree (decisions log M6-P4-M1EXIT-ROOT). It reads JSON and log inputs only. The `--host` value names the column; no x86 binary ran on the Mac.
- One run per host column. Each run also prints the `x86_64-pc-windows-msvc (CI)` column from the CI run JSON. The Windows (CI) cells and their evidence text are the same in the 3 host runs (checked by hash). The 3 exit-column runs report "Missing inputs: None". The extra Windows-as-host run lists `--cross` (item 2) and `--nesting` (item 8): Windows has no such input, by design.
- Common flags: `--base m0-exit`, `--verdicts docs/measurements/2026-11-05-spike-c.md`, `--ci-run 41864288129` with the run JSON, `--fuzz` for each counted record (section 4). The cargo-fuzz run-1 records are not given: run 2 replaces them (plan v11 §8 item 2 allows run 1 or 2; the run-1 eval_context record still holds the closed key bb32e719 as `open`, and item 5 counts the open crashes of every record given).
- The test262 checkout is the pin `d53ebc8b` (`third_party/test262.commit`). Items 3 and 4 count its files.

| Column | `--host` | `--kernels` (CI artifact) | `--cross` | `--exec`, `--exec-gc` x6, `--nesting` (CI artifacts) |
|---|---|---|---|---|
| mac a64 | aarch64-apple-darwin | `m1-exit-inputs-kernels-macos-arm64` | docs/measurements/2026-11-05-exit2-run8.json (gate window 8) | `m0-exit-inputs-macos-arm64`, `m0-exit-inputs-gc-macos-arm64-<k>-of-6` |
| linux a64 | aarch64-unknown-linux-gnu | `...-kernels-linux-arm64` | none (item 2 is N/A by design, V11-D32) | `...-linux-arm64` |
| linux x86-64 | x86_64-unknown-linux-gnu | `...-kernels-linux-x64` | docs/measurements/2026-11-05-exit2-run8-x86.json (x86 run 8, non-gate) | `...-linux-x64` |
| Windows (CI) | the second column of each run above | step `kernels` of `bench-run (windows-x64)` | N/A by design | CI jobs and steps (section 1.2) |
| extra: Windows as host | x86_64-pc-windows-msvc | `...-kernels-windows-x64` | none | `...-windows-x64` (the artifact has no nesting log) |

### 1.1 The 40 cells

P = PASS, O = OPEN, N/A = not applicable by design, NR = NOT-RUN. Source: the 4 m1-exit outputs of the final run at X = `c5588318`, 2026-11-05 UTC (scratchpad, final m1-exit columns).

| # | M1 exit item | mac a64 | linux a64 | Windows (CI) | linux x86-64 | extra: Windows as host |
|---|---|---|---|---|---|---|
| 1 | All 8 v8-v7 kernels and Octane-7 run correctly | P | P | P | P | P |
| 2 | Octane-7 >= 1.0x QuickJS; v8-v7 7-kernel >= 0.75x node --jitless | P | N/A | N/A | **O (named)** | NR |
| 3 | `language/` >= 85% | P | P | P | P | P |
| 4 | Same pass set under GC-stress with S1 on; verifier clean | P | P | P | P | P |
| 5 | Fuzzilli 100 CPU-h, cargo-fuzz 24 CPU-h, 0 unique crashes | P | P | P | P | P |
| 6 | No H-row change without an approved ADR amendment | P | P | P | P | P |
| 7 | Spike C gate green; H2, H14, H16 Frozen (H17-H19 at M1a) | P | P | P | P | P |
| 8 | Nesting chains pass cap-1 / cap+1 | P | P | P | P | NR |
| 9 | H29-H34 Frozen | P | P | P | P | P |
| 10 | SP-Facts green | P | P | P | P | P |

| Column | PASS | OPEN | N/A | NOT-RUN | FAIL | exit code |
|---|---|---|---|---|---|---|
| mac a64 | 10 | 0 | 0 | 0 | 0 | 0 |
| linux a64 | 9 | 0 | 1 | 0 | 0 | 0 |
| Windows (CI) | 9 | 0 | 1 | 0 | 0 | (in each run) |
| linux x86-64 | 9 | 1 | 0 | 0 | 0 | 1 |
| **40 cells** | **37** | **1** | **2** | **0** | **0** | |
| extra: Windows as host | 8 | 0 | 0 | 2 | 0 | 1 |

By V11-D33: 39 cells PASS or N/A, 1 named OPEN (item 2, linux x86-64). The linux x86-64 run exits 1 for that cell only; the extra Windows-as-host run exits 1 for its 2 NOT-RUN cells (items 2 and 8), by design. Extra rows (no exit-code effect), the same in all runs: the carried M0 item 5 (SP-Dispatch condition 2 on AMD EPYC 7763) OPEN; stub markers PASS; ADR Counts line PASS.

### 1.2 Evidence per item (short, from the tool text)

| # | Evidence (one line) | Input file |
|---|---|---|
| 1 | 15/15 runs, output equal (v8-v7 8/8, Octane-7 7/7) on every host; the JSON names GitHub's merge commit `4d606076`, and its 7 engine tree ids equal HEAD's (X10). Windows: step `kernels` success | CI `m1-exit-inputs-kernels-<os>`/kernels.json |
| 2 | mac: gate host m5max-macos, not provisional, both readings `pass`: Octane-7 1.438, v8-v7 1.011 (section 2); commit CF, dirty false. x86-64: "no gate verdict (the interval reads pass)" 1.375 and 0.871; "marginal (no gate host)", provisional, THP never | docs/measurements/2026-11-05-exit2-run8.json, -run8-x86.json |
| 3 | `language/` 22,969 / 23,726 files = 96.81% (>= 85%), infra 0; all files 39,426 / 53,616; 0 new failures, 0 now passing. Windows: step `test262 exec` success | CI `m0-exit-inputs-<os>`/test262-exec.json |
| 4 | 6 shards of one 6-way split: GC stress 1, S1 on, verify cadence 1, major GC every 64; files 39,389 / 53,565 + 51 skipped, infra 0; skip list 51 paths (44 at m0-exit, each added path named); 0 new failures. Windows: 6 of 6 GC-stress jobs green, step names say S1 | CI `m0-exit-inputs-gc-<os>-<k>-of-6`/test262-exec-gc.json |
| 5 | "Fuzzilli 100.7 / 100 CPU-h, cargo-fuzz 25.1 / 24 CPU-h, 0 open unique crashes" (the tool text; the sum of the 6 records is 100.67) | section 4 records; scratchpad, final m1-exit columns |
| 6 | `adr-lint --base m0-exit`: trailers, review records, Counts, path:line drift and short cites all PASS | the HEAD tree |
| 7 | the 5 Spike C rows go, Commit CF fresh; H2 (ABI core), H14 (frames core), H16 (3 parts) Frozen; H17-H19 parts due by M1 exit Frozen | docs/measurements/2026-11-05-spike-c.md; ADR Status lines |
| 8 | the 9 tests ok, 2 cargo runs, each with exit status 0. Windows: step `Run cargo test --workspace --locked` success | CI `m0-exit-inputs-<os>`/nesting.log |
| 9 | H29 Contract, H30 Model, H31 API, H32 Policy, H33 Rule, H34 Backend contract: Frozen | ADR Status lines |
| 10 | C5a, C5b, C5c go; lowering `jit, PROFILED input` | docs/measurements/2026-11-05-spike-c.md |

## 2. Gate window 8 and the x86 run 8

**Gate window 8** ran on CF on the gate host m5max-macos (Apple M5 Max, darwin-arm64), not provisional, with no agent, no build and no poller (decisions log §16.6; decisions M6-ORCH-75 to -77). Try 1 is void (an agent workflow ran in the window, rule 23); the retry gives the readings. Load guard: before 2.2, after 2.2 (limit 3) ([perf run 8](2026-11-05-perf-v0-run8.md):4).

| Reading | Net median | Interval (ranks 2 and 9) | IQR % | Rounds | Verdict | Source |
|---|---|---|---|---|---|---|
| Octane-7 geomean vs QuickJS | **1.438** | 1.432-1.443 | 0.61 | 10 | pass (threshold 1.0) | docs/measurements/2026-11-05-exit2-run8.json, `octane7-vs-qjs` |
| v8-v7 7-kernel geomean vs node --jitless | **1.011** | 1.006-1.018 | 0.90 | 10 | pass (threshold 0.75) | the same file, `v8v7-7-vs-node-jitless` |
| `kernels.mjs --check` (same build) | **15/15** | - | - | - | v8-v7 8/8, Octane-7 7/7 with node's output | decisions log §16.6 (gate window 8 kernels log; scratchpad) |
| Geomean of 14 workloads vs QuickJS | 1.436 (net) | - | - | 10 | report only | [perf run 8](2026-11-05-perf-v0-run8.md):68 |
| Geomean of 14 workloads vs node --jitless | 1.011 (net) | - | - | 10 | report only | perf run 8:68 |
| M0 exit 4 canary: richards, deltablue vs QuickJS | 1.500, 1.640 | 1.492-1.510, 1.621-1.649 | - | 10 | pass (>= 0.6) | perf run 8:74-75 |

Compared with gate window 7 (M1a report §6): Octane-7 1.412 -> 1.438; v8-v7 0.992 -> 1.011. A ratio above 1 means that ep9 is faster.

The void try 1 gave Octane-7 1.440 [1.433, 1.447] and v8-v7 1.012 [1.002, 1.020] (decision M6-ORCH-75). It is a report only, not an input.

**The x86 run 8** (plan v11 §6 W0-5, V11-D47; decision M6-ORCH-83): one remote box, AMD EPYC, 8 logical CPUs, Linux, THP never, no host file, so **provisional** by design ([x86 run 8](2026-11-05-perf-v0-run8-x86.md):3, :7; THP: docs/measurements/2026-11-05-exit2-run8-x86.json `host.transparent_hugepage`). The box is destroyed (running false).

| Reading | Net median | Interval | IQR % | Verdict | Source |
|---|---|---|---|---|---|
| Octane-7 geomean vs QuickJS | **1.375** | 1.350-1.394 | 1.91 | no gate verdict (the interval reads pass) | docs/measurements/2026-11-05-exit2-run8-x86.json |
| v8-v7 7-kernel geomean vs node --jitless | **0.871** | 0.855-0.885 | 2.30 | no gate verdict (the interval reads pass) | the same file |

Compared with x86 run 7 (M1a report §6): 1.356 -> 1.375; 0.806 -> 0.871. Both files name commit CF with `"dirty": false` and the engine build `features jit; preset jit`.

**M2 statement.** Item 2 on linux x86-64 stays OPEN "marginal (no gate host)" until a bare-metal x86-64 gate host exists (V9-D31). A remote box gives only provisional readings. **A bare-metal x86 gate host is an owner action**: the owner gets or names the machine; the M2 plan then adds its host file and its gate window.

## 3. Spike C and SP-Facts Verdict rows

Source: [docs/measurements/2026-11-05-spike-c.md](2026-11-05-spike-c.md) (the writer `spike-c verdict-rows`; `bash spikes/spike-c/gate.sh verdict`, all 27 steps PASS). Commit CF; host aarch64-apple-darwin, Apple M5 Max; the verdict step ran alone after 60 s idle at load1 2.57 (limit 3.0) (decision M6-ORCH-77).

| Spike | Condition (short) | Number | Verdict | ADRs |
|---|---|---|---|---|
| Spike C | C1: the hot sets of crypto and navier-stokes compile from live facts, output equal to T0 | stays in T0: crypto montReduce, bnpSquareTo | go | H2, H14, H16, H17, H29 |
| Spike C | C2: each guard of am3 forced alone deopts to T0, output equal, one counted exit | - | go | the same |
| Spike C | C3: the tier differential, kernels and corpus, every guard forced, 0 mismatches | 0 mismatches | go | the same |
| Spike C | C4 crypto | R 4.004 [3.894, 4.106], 15 rounds, IQR 4.5% | go | the same |
| Spike C | C4 navier-stokes | R 5.006 [4.991, 5.095], 15 rounds, IQR 1.4% | go | the same |
| SP-Facts | C5a: the lattice and ref-node checks on every compiled graph | - | go | H30, H31, H32, H33, H34 |
| SP-Facts | C5b: the metadata holds offsets only; a moved copy deopts the same | - | go | the same |
| SP-Facts | C5c: crypto from a PROFILED file, output equal | Q = t(C) / t(P) 1.001 [0.993, 1.009], 15 rounds, IQR 1.2% | go | the same |

All 8 rows are go, so V11-D29 (the C4 navier-stokes slip) does not apply, and no 09:236 reopen review runs. The m1-exit items 7 and 10 read these rows (section 1.2).

## 4. Counted fuzz

Pre-registered in the decisions log: cargo-fuzz run 2 by M6-GW8-FUZZ-CARGO-2, Fuzzilli wave 1 (cf8-fz-1..4) by M6-GW8-FUZZILLI (rule 41). The top-up cf8-fz-9 and cf8-fz-10 (335 min x 4, not in M6-GW8-FUZZILLI) is registered by M6-P5-FUZZ-TOPUP. **Rule 41 gap, named:** the top-up plan was in the orchestrator log before the dispatch, but the M6-P5-FUZZ-TOPUP row was written after the dispatch, not before it. The tool, the tree (CF), the image and the counting rules are those of M6-GW8-FUZZILLI; only the run length and the box ids differ. Results: decisions log M6-P4-FUZZ. Records: scratchpad m1m6/fuzz/counted/ (one ep9-fuzz/v1 record per campaign). CPU-h = measured seconds x workers / 3600.

| Tool | Campaign | Target or preset | Commit | Seconds x workers | CPU-h | Crashes | Record |
|---|---|---|---|---|---|---|---|
| cargo-fuzz | gh41953013001-1-eval_context | eval_context | 3173265c | 7,547 x 4 | 8.3856 | 0 | run2/fuzz-record-eval_context |
| cargo-fuzz | gh41953013001-1-frontend_compile | frontend_compile | 3173265c | 7,531 x 4 | 8.3678 | 0 | run2/fuzz-record-frontend_compile |
| cargo-fuzz | gh41953013001-1-function_ctor | function_ctor | 3173265c | 7,534 x 4 | 8.3711 | 0 | run2/fuzz-record-function_ctor |
| **cargo-fuzz total** | | | | | **25.12** (target 24) | **0** | |
| Fuzzilli | fz-20261105-cf8-fz-1-030238 | jit | CF | 12,601 x 4 | 14.0011 | 0 | cf8/fuzzilli-cf8-fz-1.json |
| Fuzzilli | fz-20261105-cf8-fz-2-030308 | jit | CF | 12,600 x 4 | 14 | 0 | cf8/fuzzilli-cf8-fz-2.json |
| Fuzzilli | fz-20261105-cf8-fz-3-030337 | jit + GC stress | CF | 12,601 x 4 | 14.0011 | 0 | cf8/fuzzilli-cf8-fz-3.json |
| Fuzzilli | fz-20261105-cf8-fz-4-030432 | jitless | CF | 12,600 x 4 | 14 | 0 | cf8/fuzzilli-cf8-fz-4.json |
| Fuzzilli | fz-20261105-cf8-fz-9-105950 | jit | CF | 20,101 x 4 | 22.3344 | 0 | cf8/fuzzilli-cf8-fz-9.json |
| Fuzzilli | fz-20261105-cf8-fz-10-110020 | jitless | CF | 20,100 x 4 | 22.3333 | 0 | cf8/fuzzilli-cf8-fz-10.json |
| **Fuzzilli total** | | | | | **100.67** (target 100) | **0** | |

- cargo-fuzz run 2 ran on 3173265c, a docs-only child of CF; its `crates/` and `fuzz/` trees equal CF's (decision M6-ORCH-87). libFuzzer exit 0 and 0 slow units on all 3 targets. The run-1 timeout bb32e719 did not come back, so the triage ledger closes it (scratchpad m1m6/fuzz/ledger.md).
- Fuzzilli ran on an image made from CF (the smoke box read commit CF, dirty false). Each record says `counted: true`, `dirty: false`. Coverage grew on all 4 wave-1 boxes (decision M6-ORCH-95) and on the 2 top-up boxes (fz-9 7.71% -> 15.71%, fz-10 7.16% -> 14.85%; scratchpad top-up log).
- Wave 2 (cf8-fz-5..8) stopped at its start by a user rule on remote CPU hours (decision M6-ORCH-96): no wave-2 record counts. The one top-up (cf8-fz-9, cf8-fz-10) ran after the month-6 tasks ended (decision M6-ORCH-103; decisions log M6-P5-FUZZ-TOPUP). A smoke box read commit CF, dirty false; every top-up box is destroyed and reads `running: false` (scratchpad top-up log).
- The final run reads the 9 records: item 5 PASS in every column (section 1.1).

## 5. FREEZE-M1

**The one Frozen step at M1 exit** is aa3807aa (2026-11-05, the orchestrator; decisions log M6-P4-FROZEN). It froze 31 ADRs (whole or by the parts that are due at M1 exit) and the M1 exit parts of H17 and H18. The gate check passed first (the 8 Verdict rows go, section 3). The orchestrator wrote the change, so it did not approve it (README §3.2). APPROVER-M6 (the GC role chain, plan v11 V11-D25) approved the 33 Frozen sets in 8e50ed4f (2026-11-05), held none (decisions log M6-P4-RECORDS-W2). The W2 reviewer wrote row 1 (3dd4f46c), the fixer rows 2-4 (215cf58a, 10d35fc6).

| ADR | Part Frozen | Date, milestone | Change-log row (Frozen step) | Approver, Approval row |
|---|---|---|---|---|
| H1 Front-end boundary | whole | 2026-11-05, M1 exit | H01:110 | APPROVER-M6, H01:156 |
| H2 Bytecode ABI | ABI core | 2026-11-05, M1 exit | H02:141 | APPROVER-M6, H02:166 |
| H3 TS runtime semantics | header fields | 2026-11-05, M1 exit | H03:94 | APPROVER-M6, H03:123 |
| H4 Lazy compile contract | whole | 2026-11-05, M1 exit | H04:115 | APPROVER-M6, H04:162 |
| H5 Always-linked compiler | whole | 2026-11-05, M1 exit | H05:95 | APPROVER-M6, H05:134 |
| H6 Value encoding | whole | 2026-11-05, M1 exit | H06:107 | APPROVER-M6, H06:134 |
| H7 Address space and code range | whole | 2026-11-05, M1 exit | H07:122 | APPROVER-M6, H07:159 |
| H8 Object header and ShapeId | whole | 2026-11-05, M1 exit | H08:168 | APPROVER-M6, H08:204 |
| H9 Pages and side metadata | whole | 2026-11-05, M1 exit | H09:114 | APPROVER-M6, H09:191 |
| H10 Rooting | whole | 2026-11-05, M1 exit | H10:126 | APPROVER-M6, H10:195 |
| H11 Barrier and atomics | barrier contract | 2026-11-05, M1 exit | H11:176 | APPROVER-M6, H11:284 |
| H12 No address identity | whole | 2026-11-05, M1 exit | H12:113 | APPROVER-M6, H12:147 |
| H13 Snapshot-stable indices | rules | 2026-11-05, M1 exit | H13:128 | APPROVER-M6, H13:186 |
| H14 Frames and dispatch | frames core | 2026-11-05, M1 exit | H14:191 | APPROVER-M6, H14:277 |
| H15 Interpreter dispatch | dispatch core | 2026-11-05, M1 exit | H15:163 | APPROVER-M6, H15:193 |
| H16 ICs and feedback | feedback core, safety rules, fact meanings | 2026-11-05, M1 exit | H16:303 | APPROVER-M6, H16:521 |
| H17 Object layout | Dictionary v1, Accessor ICs, Transition table form, Index keys on prototypes (layout core: 2026-11-02, M1a) | 2026-11-05, M1 exit | H17:265 | APPROVER-M6, H17:465 |
| H18 Elements kinds | FactSnapshot kind list (elements core: 2026-11-02, M1a) | 2026-11-05, M1 exit | H18:174 | APPROVER-M6, H18:295 |
| H21 Shape transitions | Design | 2026-11-05, M1 exit | H21:122 | APPROVER-M6, H21:186 |
| H22 Isolates and realms | whole | 2026-11-05, M1 exit | H22:152 | APPROVER-M6, H22:250 |
| H23 Limits and failure policy | rules | 2026-11-05, M1 exit | H23:210 | APPROVER-M6, H23:408 |
| H24 Debug and profiling hooks | rules | 2026-11-05, M1 exit | H24:127 | APPROVER-M6, H24:221 |
| H25 Sandbox readiness | whole | 2026-11-05, M1 exit | H25:130 | APPROVER-M6, H25:240 |
| H26 Toolchain and Unicode | whole | 2026-11-05, M1 exit | H26:100 | APPROVER-M6, H26:134 |
| H27 Compiler features | whole | 2026-11-05, M1 exit | H27:122 | APPROVER-M6, H27:146 |
| H28 Platform layer | whole | 2026-11-05, M1 exit | H28:151 | APPROVER-M6, H28:214 |
| H29 Deopt contract | Contract | 2026-11-05, M1 exit | H29:150 | APPROVER-M6, H29:254 |
| H30 IR fact model | Model | 2026-11-05, M1 exit | H30:128 | APPROVER-M6, H30:170 |
| H31 Fuse registry | API | 2026-11-05, M1 exit | H31:184 | APPROVER-M6, H31:338 |
| H32 Guard failure policy | Policy | 2026-11-05, M1 exit | H32:124 | APPROVER-M6, H32:157 |
| H33 AOT feedback rule | Rule | 2026-11-05, M1 exit | H33:113 | APPROVER-M6, H33:154 |
| H34 Relocation kinds | Backend contract | 2026-11-05, M1 exit | H34:132 | APPROVER-M6, H34:174 |
| H36 FP semantics | whole | 2026-11-05, M1 exit | H36:155 | APPROVER-M6, H36:198 |

Frozen before M1 exit, no change of Status in month 6:

| ADR | Part Frozen | Date, milestone | Approver |
|---|---|---|---|
| H17 | layout core | 2026-11-02, M1a (b5c8ba44) | APPROVER (DOCS chain), 0424ba57 |
| H18 | elements core | 2026-11-02, M1a (b5c8ba44) | APPROVER (DOCS chain), 0424ba57 |
| H19 | strings core | 2026-11-02, M1a (b5c8ba44) | APPROVER (DOCS chain), 69ad48c0 |
| H20 | core (part 2 append-only) | 2026-10-26, M0 exit (b498a01) | second reviewer agent, M3-P5-H20-APPROVAL |

Not Frozen: H35 AOT unit format stays Accepted (its parts freeze in the A lane); H37 is Not started.

**Counts** ([README](../adr/README.md):170, from the Status line of each ADR file): **0 Draft, 1 Not started, 1 Accepted, 35 Frozen** (37 ADRs). At M1a: 7 Draft, 1 Not started, 25 Accepted, 4 Frozen. m1-exit reads the Counts line as PASS (extra row).

```text
 Not started  H37                                                        (1)
 Accepted     H35                                                        (1)
 Frozen       H1-H34, H36 (whole, or by part; parts with a later freeze
              point or with no freeze point stay open)                   (35)
```

The approved records at HEAD: `adr-lint --base m0-exit` and `--base m1a` both 5/5 PASS, 0 record gaps (decision M6-ORCH-106; m1-exit item 6 PASS, section 1).

## 6. Frozen-part rewrites and notes between M1a and M1 exit

README §3.3 (the 2026-11-02 note, plan v11 inputs #31, DOC13-6): a rewrite of a part that is already Frozen, dated before the M1 exit tag, is not counted by G8 and is not pre-freeze churn; this report lists it by name. Each took the README §3.2 Frozen-part route: the review record, a benchmark statement, a second reviewer agent from another plan role and a dated decisions-log line.

### 6.1 Rewrites of a Frozen part

| ADR:line | Date | Part (Frozen at) | What changed (short) | Route evidence | Decisions log |
|---|---|---|---|---|---|
| H17:257 | 2026-11-02 (M6-P0 VM6) | layout core (M1a) | OBJECT_LAYOUT 5 -> 6; item (3): the `flags` sentence leaves the layout core; bits 4-30 go to the new part "shape flag bits (append-only)" (V11-A9). The core keeps the word, bits 0-3 and bit 31 | benchmark: t0-loop, t0-call and richards within 0.02%; `disasm-check --all-targets` equal on 5 targets; Approvals H17:420 (orchestrator) and H17:421 (RECORDS-M6, the second reviewer) | M6-P0-ORCH-H17RW |
| H20:298 | 2026-11-04 (M6-P2 fixer, 0078fd58) | core, the `check_tables` rule H20:68 (M0 exit) | `check_tables` now refuses a table that makes `IntrinsicId::RealmFuses` (id 72): "[[RealmFuses]] is VM-internal". Before, it accepted it | the new test fails at the parent; `ep9_vm` and `ep9_builtins` 1,275 pass; `ep9 -e 0` instructions retired 22,739,960 -> 22,721,228 (-0.08%); second reviewer H20Q (4fa8072f); Approval c944aa61, H20:497 | M6-P2-H20-RW |
| H16:304 | 2026-11-05 (M6-P5, MODES-CHECK-w28 MC-w28-2) | feedback core (M1 exit, aa3807aa) | The appends to the closed lists of the FactSnapshot schema (`crates/ep9_facts/tests/golden/schema.txt`) move out of "feedback core" into "IC lists (append-only)" (before, H16:237 called every FACT_SCHEMA bump a "feedback core" change): a FACT_SCHEMA bump with only such an append is a fill when every existing check, test and golden file keeps its result (README §3.3). The typed getter and setter facts get new Access tags 10 and 11 | text only, no build or run can differ (benchmark statement); second reviewer the M1-CHECK role (75fb0d32, round 2 b14ecd01: Confirmed); Approval ae834773, H16:529 | M6-P5-H16-RW |

Until the routed test change MC-P5-FACTTOP (M2) lands, an append that changes a test result at CF stays a rewrite (H16:304; section 7).

### 6.2 Clarify notes on a Frozen part

| ADR:line | Date | Part (Frozen at) | What changed (short) | Approval |
|---|---|---|---|---|
| H17:258 | 2026-11-03 (M6-P0 fixer) | layout core (M1a) | 3 moved `path:line` cites; the layout-core row says OBJECT_LAYOUT 6 (was 5), cite `versions.rs` 58; no fact changes | orchestrator, H17:426 |
| H18:171 | 2026-11-03 (M6-P0 fixer) | elements core (M1a): cites only | 9 moved `path:line` cites, same line text | orchestrator, H18:280 |
| H17:266 | 2026-11-05 (M6-P5, MC-w28-2) | shape flag bits (append-only; no freeze point) | dated note: a FactSnapshot carries shape flags only in its `ShapeFlags` field, a u8 (bits 0-7) | APPROVER-M6, ae834773, H17:471 |
| H32:125 | 2026-11-05 (M6-P5, MC-w28-V1) | Policy (M1 exit) | dated note at H32-Q5: a T1 install gets a vector only at one of the 4 T0 moments of the code at CF | APPROVER-M6, 79bc83fe, H32:162 |

The H16:236 and :237 notes of M6-P5 are the text of the H16:304 rewrite row (decision M6-ORCH-101). H17:266 is on an H17 part with no freeze point; the list keeps it because it is one of the M6-P5 Frozen-part rows (MC-w28-2), with H16:304 and H32:125.

### 6.3 The build-or-move check (V11-D24)

The read-only check at 1a6272c0 (decisions log M6-P3-BOM; decision M6-ORCH-49): every part due at M1 exit is built, and the code and every test that its Check cell names exist: H17 "Dictionary v1", "Accessor ICs", "Transition table form", "Index keys on prototypes"; H18 "FactSnapshot kind list"; H2 whole (then split: "ABI core" at M1 exit and "opcode and section appends (append-only)" with no freeze point, decisions log M6-P4-H02-RW); H14 "frames core"; H16 "feedback core", "safety rules", "fact meanings". After 1a6272c0 the code commits change only tests, `spike-c` (`verdict.rs`) and a comment in `tools/bench/cross/run.mjs`; no part changes its built state.

**Freeze-point moves: one.** H18 "extra element kinds" (NONEXTENSIBLE, SEALED, FROZEN) moves from M1 exit to **M2 exit**, flags first (plan v11 V11-D6; H18:87). It is not built. No other part moved.

Not a move: STR4 (lazy intern, an H19 "strings core" rewrite) did not run, because the V11-D11 intern measure is below its threshold (decisions log M6-P0-V11-D11). So H19 "strings core" has no rewrite after M1a.

## 7. Open to M2

### 7.1 Named open items

Only the first row is an exit exception (V11-D33). The other rows have no exit cell, or they must close before the `m1-exit` tag.

| Item | State | Why | Owner, due |
|---|---|---|---|
| Item 2, linux x86-64 | OPEN "marginal (no gate host)" | the one named cell of V11-D33 (V9-D31); x86 run 8 is provisional (section 2) | owner action: a bare-metal x86-64 gate host; then M2 |
| H18 "extra element kinds" | not built; freeze point M2 exit | V11-D6 (section 6.3) | the M2 extra-kinds builder (also MC-w23-19) |
| H21 "Design" | Frozen at M1 exit with no check yet; the check comes at S2b (M5) | no gate reads the design before M5 (H21:61; decisions log M6-P4-CALLS (6)) | the S2b lane |
| Carried M0 item 5: SP-Dispatch condition 2 on AMD EPYC 7763 (H15-Q5) | OPEN (extra row, no exit effect) | no non-void EPYC 7763 reading ([M0 report](2026-10-26-m0-milestone-report.md) §1) | due M2 exit (V9-D30), by the H15 Freeze-table part "x86-64 Zen 3 condition" |

### 7.2 Routed rows due M2, A2, A3, M7 (open)

Source: the routes list (scratchpad m1m6/routes.tsv; plan v11 rule 47, §14). 50 rows are open with these due tags: M2 15, A2 20, A3 4, M7 11. (Other open rows: A4 4, A1 1, M3 1, "after M1" 1.)

**M2 (15)**

| Id | One line |
|---|---|
| MC-P4-SLOTABI | the ABI fingerprint does not hold the FeedbackSlotKind table, so a slot-kind append without an ABI_VERSION bump passes (first code phase after m6-p5) |
| MC-P5-FACTTOP | make the `ep9_facts` tests append-safe (no last-tag pin, no count pin, no next-free probe), so a closed-list append keeps every test result |
| MC-P4-SLOTTOP | no test probes the top slot-kind byte 0xff |
| MC-P1-LAYOUTX | append 3 `ep9_vm::layout` exports and drop the spike-c `HAND_FACTS_M6` copies (READ_SET 69 -> 72) |
| MC-w23-19 | `generalize_site` stops at DICTIONARY: choose the birth-kind rule when the extra kinds are built |
| MC-P3-ELDOC | `crates/ep9_vm/src/elements.rs` doc comment still says FACT_SCHEMA 1 |
| MC-P3-OPAGREE | the access-kind agreement test also checks the operands, with a negative control |
| MC-P3-KEYEDKIND | the writer-coverage test checks each of the 5 keyed slot kinds alone |
| MC-SPEC-BIGPOLL | long BigInt work does not poll for an interrupt (H23); `toString(2)` upper-bound guard is 1 unit strict |
| MC-w28-5 | write down that H8 word0 flag bits 10-15 and H25 inventory rows are fills when used |
| MC-w28-10 | doc 13 §5.4 Kind A: add rows for the H2 appends part and H16 "declared fields (shrink-only)" |
| MC-P5-VECMOM | H14:42, H14:86 and H16 K6 name 2 of the 4 feedback-vector moments at CF: add dated clarify notes |
| MC-P5-FBDOC | `crates/ep9_vm/src/feedback.rs` module doc table: the function-0 and call-budget rows |
| V11IN-12 | plan v11 inputs #12: regress patch 0005 (resumable scanner), for the M2 RegExp chain (V11-D39; no M1 item) |
| V11IN-22 | plan v11 inputs #22: the extref-lint evasion forms, after M1 (V11-D39) |

**A2 (20)**

| Id | One line |
|---|---|
| MC-w22-16.2 | never bake `is_routed` / `is_callable` into a unit (v11in #39) |
| MC-w22-17.3 | root slots append-only from A2 (C-18; v11in #40) |
| MC-w22-21.3 | DEOPT_META 0 -> 1 at A2 start, with the H35 container that carries VM_LAYOUT (V11-D12) |
| MC-w22-O5 | a run with no unit pays nothing: a perf-ci-v0 check (v11in #42) |
| C-w22-1 | align unit sections to 64 KiB pages, or copy at load |
| MC-w23-11.4 | the unit-format part of the intern-at-load cost (C-11, C-17; v11in #48) |
| MC-w23-15 | BTI: a "has pads" flag, PROT_BTI only when set, a pad at every indirect entry |
| MC-w21-12 | a unit function has no IC store for its first calls: measure the cost |
| MC-w20-4.3 | native literal and CloneObject paths: ALLOC_SITE birth and count, CLONE widen, or named hooks |
| M5-P0-ORCH-6 | the `ep9_unit` header VM_LAYOUT field (H35-Q10), with the DEOPT_META 1 bump |
| MC-p0-cx-1 | under hybrid, `select()` can pick an AOT object whose lowering is jit: the A2 loader adds the skip and a test |
| MC-p0-cx-2 | counter sessions are not modelled in `select()`: choose and build at A2 |
| MC-w24-3 | `Policy::of(Preset)` is a second preset table: the first caller reads the ExecPolicy fields |
| MC-P1-JIT5 | a loop guard that exits at entry on each call: recompile on exit or loop peeling (with EXITED) |
| MC-w25-9 | the `===` atom arm is one more reader of H19 "one atom per text": unit load keeps one intern table |
| MC-w27-7a | HostEnsureCanCompileStrings on an eval-cache hit: the check part |
| MC-w27-6 | the Root golden is equal in the jit and no-jit builds; the build id names no features or tree state |
| MC-w28-7 | EXITED and a T2 loop-speculation bit: their home in the H29-Q5 counters and the record slot field |
| MC-w28-8.1 | report deopt metadata bytes per unit |
| MC-w28-9.1 | build flag: do not opt in to CET/GCS shadow stacks (lazy deopt swaps return addresses) |

**A3 (4)**

| Id | One line |
|---|---|
| MC-w22-13.2 | does C-21 name the feedback mode in the merge input (v11in #38) |
| MC-w22-O1.2 | the differential runs each pass set under target = jit too |
| MC-w23-12 | `--profile-out` sets feedback full on every preset |
| MC-w23-13.2 | one snapshot loaded under 2 seeds |

**M7 (11)**

| Id | One line |
|---|---|
| MC-w22-O2.2 | H30-Q6: class, prototype, builtin, module namespace and literal identity are fuses or guarded facts |
| MC-w23-14 | string cells change in place: an H30 effects line and a T2 differential test |
| MC-w23-16 | module start order is a whole-program fact (H30-Q6), with a test |
| MC-w24-6.2 | the T2 test for MC-w24-6.1 |
| MC-P1-APPLY | raytrace: host `Function.prototype.apply` cost: a builtin id for %apply% and a frame push from `arguments` |
| MC-w27-3a | the code part of MC-w25-3 (0x22 lists: the site's this; a dead holder misses) |
| MC-w27-7 | an inlined call/apply gives the H20-Q3 host path result and reads only its own site's feedback |
| MC-w27-9 | the tier manager keeps each compile refusal per function and does not retry on each hot trigger |
| MC-w15-18 | Proven and Predicted become distinct Rust types in `ep9_opt`; then an A3 random-profile run |
| MC-w27-5a | the T2 pin of the context ops 0x1e and 0x20 for the H32:61 hoist rule |
| MC-w28-6 | H6 freezes the immediate list; 0x22 is both the array hole and TDZ |

Plan v11 inputs rows #12 and #22 have the routes rows V11IN-12 and V11IN-22 (due M2, above). Row #4 (the verifier at T2) and row #33 (doc 13 room-note tests at T1, A2, T2) are lane work of M5 to M7.

### 7.3 Other open items for the M2 plan

- The x86-64 readings of run 8 (1.375 and 0.871) are the input for the M2 Linux x64 gate (section 2).
- The SPEC lane side branch `m1m6/spec` merges after the `m1-exit` tag (plan v11 §8 item 4); its open records rows and Approvals are listed in decision M6-ORCH-86. At the merge, the spec-temporal H16 row changes kind from rewrite to fill (a shrink of "declared fields (shrink-only)") and its H16:224 text merges (routes MC-w28-3).

## 8. Statement: M1 holds, the M2 plan starts

The final m1-exit run at X = `c5588318` (the 4 columns with the cf8-fz-9 and cf8-fz-10 records added) gives item 5 PASS in all 4 columns: 39 cells are PASS or N/A and the one other cell is the named item 2 linux x86-64 OPEN. By V11-D33, **M1 holds at `c5588318`**: the H-row core of the engine is Frozen (35 of 37 ADRs), the Spike C and SP-Facts gates are go, gate window 8 passes on the gate host, and the counted fuzz has 0 crashes. Then the tag `m6-p5` and the annotated tag `m1-exit` go on the last report commit (the M1-CHECK fixes) after its CI is green (plan v11 §8 item 4), and **the M2 plan (plan v12) starts** with section 7 as its input.
