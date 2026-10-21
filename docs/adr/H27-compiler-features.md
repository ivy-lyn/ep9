# H27: Compilers as Cargo features

| Field | Value |
|---|---|
| Status | Draft (2026-10-21). As built in M0 wk4: `aot` on `ep9_unit` (empty module), `cargo xtask feature-matrix`, dep-guards checks 9 and 10 (see "As built"). M0 month 2 P0: `ep9_vm` features `spike-native` and `ic-stats`, dep-guards checks 11 and 12 |
| Fix by | M0 (wk4) |
| Freeze point | M1 exit |
| Origin | v1#25; plan status E (extended: `aot`) |
| Crates | `ep9_cli` (default `jit`), `ep9_vm` (`jit`; `spike-native` and `ic-stats`, never in `ep9`), `ep9_platform` (`jit`, `unwind`), `ep9_unit` (`aot`, empty in M0); later `ep9_masm`, `ep9_baseline`, `ep9_opt`, `ep9_regexp` (RX2 under `jit`), `ep9_aot`, `ep9c` |
| Owner (plan role) | JIT/platform lead (`jit`); AOT senior (`aot`) |
| Validated by | SP-JIT-mem (passed on 4 targets, 2026-10-21); dep-guards checks 5, 9, 10, 11, 12 and 13; `cargo xtask feature-matrix` as a build-test step on 4 runners (M0 wk4) |
| Plan sources | [02 §6 H27](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [08 §2](../plan/08-repo.md#2-crates), [08 §3](../plan/08-repo.md#3-binaries), [08 §7](../plan/08-repo.md#7-dependency-guards-and-xtask-checks), [04 §5](../plan/04-execution.md#5-jit-infrastructure-and-the-relocatable-macroassembler) |

## Context

- G9: a first-class jitless build and an AOT-jitless mode. D11 = yes: AOT-jitless ships in v1. D16: iOS (no runtime codegen) only through this split.
- H5: the bytecode compiler is always linked, because eval, `new Function` and `toString` work in every mode. Hermes' lean VM without its compiler aborts on eval [R14].
- Cargo builds every package once per build, with the union of the features that the selected packages ask for. So `cargo build --workspace` turns `jit` on in the jitless graph too, and hides a leak.
- Depends on: H5, H28 (`jit` and `unwind` modules). Depended on by: H34, H35 (unit writer under `aot`), 10-quality §1 (test262 per mode).

## Decision

`jit` (runtime codegen) and `aot` (unit emission). The jitless build keeps T0, the compiler, the unit loader and the deoptimizer (02 §6).

```text
 ep9_cli  default = ["jit"]
   jit ──> ep9_vm/jit ──> ep9_platform/jit ──> ep9_platform/unwind
 ep9_unit  aot = []          (empty module in M0; the writer comes at A2)
 later: jit -> ep9_masm, ep9_baseline, ep9_opt, ep9_regexp RX2
        aot -> ep9_masm unit finalize, AOT emit in ep9_baseline/ep9_opt, ep9_aot, ep9c
```

| Build | Command | Contains | Leaves out |
|---|---|---|---|
| `ep9` (JIT) | `cargo build -p ep9_cli` | default `jit` | unit writer, `ep9_aot` |
| `ep9` jitless | `cargo build -p ep9_cli --no-default-features` | T0, compiler, unit loader, deoptimizer | every runtime codegen |
| `aot` (M0 test build) | `cargo build -p ep9_unit --features aot` | the empty `aot` module | — |

- `ep9_platform` has `jit = ["unwind"]` and `unwind = []` since M0 wk3 (`crates/ep9_platform/Cargo.toml:27-34`). `unwind` is separate because the jitless build needs it for AOT units (A2). Unit tests build both modules under `cfg(test)` (H28).
- Week 4 (P0) adds `jit` to `ep9_vm` and `ep9_cli` (default) and `aot` to the new `ep9_unit` crate. Nothing else in M0 sits under `aot` (decision W4-FEATURES).
- `ep9_vm`'s `become` feature selects a dispatch expansion (H15, nightly only). It is not a compiler feature.
- `ep9_vm` has two non-default features for measurement, never in any `ep9` build (M0 month 2):
  - `spike-native = ["jit"]`: Spike B's native entry in T0's call path. Only `spikes/spike-b` turns it on. That spike has its own workspace and lock, so its assembler (`dynasm`, `dynasmrt` `=5.1.0`, MPL-2.0) never enters the root graph or `cargo deny` (root workspace only). Adding `dynasm` to the root graph needs a per-crate exception in `deny.toml` (lines 13-14) and a row here.
  - `ic-stats = []`: inline-cache hit and miss counters for reports (H16).

### As built (M0 wk4)

| Item | Detail |
|---|---|
| `aot` | `crates/ep9_unit/Cargo.toml`: `aot = []`, gating an empty `mod aot {}`. The unit header codec and the load check (H35 S6) are loader code, so they are in every build. Tests build units with a test-only encoder (`crates/ep9_unit/tests/common/mod.rs`) |
| `cargo xtask feature-matrix` | `tools/xtask/src/features.rs`. 9 cells: the 3 builds above x `build`, `clippy --all-targets -- -D warnings`, `test`. Every command has `--locked` and `-p`, never `--workspace`. It runs every cell even after a failure, then prints PASS or FAIL per cell. Exit 0 all pass, 1 any fails, 2 usage error or cargo cannot start. Options: `--set`, `--step`, `--target` (a cross target allows `build` and `clippy` only), `--dry-run` |
| Guards 9 and 10 | They read each package's enabled features from `cargo tree --target all -e normal,build,dev -f '{p}\|{f}'` (dev edges included, so the graph is the test build's). `cargo tree -e features` does not show the features that the root package's own `[features]` table turns on, so a grep of it cannot see `jit` even in the default build (decision W4-GUARD-FEATURES) |
| First local runs | macOS arm64 (M5 Max), warm cache. Run 1: 95.5 s wall, `aot` clippy FAIL (a lint in the new `ep9_unit` tests, fixed before merge). Run 2: 65.6 s wall, 9 of 9 PASS. `--target x86_64-pc-windows-msvc --step clippy`: 3 of 3 PASS |
| CI | A step in the build-test job on 4 runners (P3-CI, M0 wk4) |

## Rules

| Rule | Check | From |
|---|---|---|
| The jitless `ep9` links no `ep9_masm`, `ep9_baseline` or `ep9_opt` | dep-guards check 5 (`.github/scripts/dep-guards.sh:144-149`) | M0 wk1 |
| The jitless `ep9` has no `ep9_platform` feature `jit` | dep-guards check 9 (`.github/scripts/dep-guards.sh:160-166`), with a control: the default build must show `jit` | M0 wk4 |
| No `ep9_unit` feature `aot` in the `ep9_cli` graph, default or jitless | dep-guards check 10 (`.github/scripts/dep-guards.sh:168-178`), with a control: `-p ep9_unit --features aot` must show `aot` | M0 wk4 |
| No `ep9_vm` feature `spike-native` or `ic-stats` in the `ep9_cli` graph, default or jitless | dep-guards check 11 (`.github/scripts/dep-guards.sh:180-189`), with a control: `-p ep9_vm --features spike-native,ic-stats` must show both | M0 month 2 |
| H5: `ep9` (the runtime crate, `ep9_cli` today) links the bytecode compiler `ep9_frontend` in its normal graph, default and jitless | dep-guards check 12 (`.github/scripts/dep-guards.sh:191-213`), with two controls: `ep9_vm`'s graph must not show `ep9_frontend` (`ep9_vm` reaches the compiler only through the `IsolateConfig::compiler` hook), and `ep9_builtins` (a dev-only edge) must not pass the query | M0 month 2 |
| Release builds abort on a panic: `[profile.release]` of Cargo.toml has `panic = "abort"` (dev-opt inherits it), and no other profile table and no `.cargo/config.toml` sets `panic` (H23) | dep-guards check 13 (`.github/scripts/dep-guards.sh:215-277`): flags every TOML spelling of a `panic` key (quoted, dotted, inline table, `\u` escape) in Cargo.toml, `.cargo/config.toml` and `.cargo/config`; controls: 9 samples must be flagged (:244-252), `panic_x`, `no-panic` and a comment line must not (:254-256), and the Cargo.toml rule accepts only `[profile.release] panic = "abort"` (:257-260) | M0 month 2 |
| Each build is built, linted (`-D warnings`) and tested per package, never with `--workspace` | `cargo xtask feature-matrix`; CI build-test step on 4 runners | M0 wk4 |
| Code under `jit` compiles out without the feature | `#[cfg(any(feature = "jit", test))]` (for example `Reservation::reserve_code_range_at`, `crates/ep9_platform/src/va.rs:135`) | M0 wk3 |
| The jitless build passes the same test262 set as the JIT build | test262 per mode (10-quality §1) | when T1 exists (M1+) |

## Consequences

- **Gains.** One code base gives the JIT `ep9`, the jitless `ep9` and later `ep9c`. The jitless build is a correctness oracle for the JIT tiers. iOS needs no fork.
- **Costs.** Three builds per target in CI. Every `cfg(feature)` needs a guard or a matrix entry, or it rots. Workspace-wide commands (`cargo test --workspace`) never prove the jitless graph.
- **Follow-ups.** `ep9c` and its feature set (A1); the test262 `--force-mode` builds (A3); who turns on `ep9_platform/unwind` in the jitless build (A2).

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R06] | V8 jitless and JSC LLInt-only runs show what an interpreter-only build reaches; W^X toggle cost on macOS | [06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R14] | Static Hermes: the compiler is 2.4 MB of a 6.0 MB runtime; the lean VM without it cannot run eval | [14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [M-JIT] | SP-JIT-mem: executable memory, W^X, unwind and backtrace pass on 4 targets under `ep9_platform/jit` | [2026-10-21-sp-jit-mem.md](../measurements/2026-10-21-sp-jit-mem.md) |

## Validation

- **SP-JIT-mem** ([09 §4](../plan/09-milestones.md#4-spikes), M0 wk3): on fail it reopens H28 and H27. **Result: PASS on 4 targets** ([M-JIT]; again in CI run 39518270586 on the week-4 base tree).
- Week-4 check: "`aot` feature builds on 4 targets" ([09 §8](../plan/09-milestones.md#8-m0-first-month-week-by-week)): the feature-matrix step on 4 runners.
- dep-guards on every PR.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H27-Q1 | When does `ep9c` arrive, and with which features? | A1 | `ep9c` = `aot` only, no `jit` |
| H27-Q2 | test262 `--force-mode` builds: `jit` + `aot` and `aot` without `jit` | A3 | Two test262 builds, as 08 §2 says |
| H27-Q3 | Which crate turns on `ep9_platform/unwind` in the jitless build? | A2 | The `ep9_unit` loader (unit unwind registration) |
| H27-Q4 | Does the `aot` matrix entry stay on `ep9_unit`? | A2 | Move it to `ep9c` when `ep9c` exists |

## Reopens if

- SP-JIT-mem fails on a tier-1 target (09 §4 names H28 and H27).
- A guard cannot express a feature rule (for example Cargo changes how it reports features).
- D11 or D16 changes (11-decisions "Affects").

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft from 02 §6, 08 §2-§3 and §7, the `ep9_platform` features and the week-4 feature plan (M0 wk4 plan, task P0-DOCS 3; decision W4-FEATURES) | P0-DOCS agent |
| 2026-10-21 | rewrite (kind set 2026-10-22; was fill) | As built in M0 wk4: the `feature-matrix` cells, options and exit codes; how guards 9 and 10 read features (with positive controls), replacing the plan's `-e features` wording in the Rules table; the check 5 cite moved to lines 144-149 after the P0 guard rewrite; the empty `aot` module and the test-only unit encoder. No rule changes. Decision W4-AOTU-MATRIX | P1-AOTU agent |
| 2026-10-21 | fill | M0 month 2 P0: the `ep9_vm` features `spike-native` and `ic-stats` (non-default); the `spikes/spike-b` workspace with `dynasm` outside the root graph; two new Rules rows: check 11 (neither feature in `ep9`) and check 12 (H5: `ep9` links `ep9_frontend`), each with a control. Existing rules and checks keep their results | P0-INFRA agent (M0 month 2) |
| 2026-10-21 | rewrite (kind set 2026-10-22; was clarify) | Check 12 reads only cargo tree's stdout and needs an `ep9_cli` node; a second control (`ep9_builtins`, dev-only edge) must not pass. Before, `warning: nothing to print.` on stderr counted as a match. Its `path:line` range is now 191-213 | M2-P0 integration fixer |
| 2026-10-22 | clarify | Kind cells set to rewrite (README §3.3) in two rows. 2026-10-21 "As built in M0 wk4" (P1-AOTU): it replaced the check text of the guard 9 Rules row (`-e features`) and added controls to rows 9 and 10. 2026-10-21 "Check 12 reads only cargo tree's stdout" (M2-P0 fixer): check 12 now rejects what it accepted (`warning: nothing to print.` on stderr), and its Rules row gained a second control | P2-ACCEPT-1 (kinds); P2 review fixer (Codex A2: kind cells) |
| 2026-10-22 | clarify | Row added for bb5967e (2026-10-21, P1-CODE-RANGE), which had none: the "Code under `jit` compiles out" Rules row names `Reservation::reserve_code_range_at` as its example, since `Cage::reserve_jit` is gone. The rule and its check are unchanged. Later `va.rs` line moves are ADRLINE ref fixes | P2-ACCEPT-1 |
| 2026-10-22 | fill | M0 month 2 P2 (M2-P2-GUARD13-H27): new Rules row for dep-guards check 13 (release builds abort on a panic; H23 and M2-ABORT-SCOPE), with its control. Text from the P2-INFRA2 report. Existing rules and checks keep their results | P2 integration fixer |
| 2026-10-22 | rewrite | Check cell of the guard-13 Rules row (M2-P3-H27-65; closes M2-P2-OPEN-H27-65): it described the old single control ("the scan must find the key in a sample `[profile.dev]` table"). The P2 review fix I7 (06805b7, M2-P2-I7) made the scan flag every TOML spelling of a `panic` key, added `.cargo/config`, and grew the one control sample to 9 must-flag samples, 3 must-not lines and two tests of the Cargo.toml rule; the cell now says so. The kind is rewrite because a Check cell changes (as M2-P2-FIX-H03-KIND) | P3-ACCEPT-2 |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P2-ACCEPT-1 (did not write the ADR) | Decision = the 02 §6 H27 row. Rules checked against `dep-guards.sh` (checks 5 and 9-12, their line ranges), `tools/xtask/src/features.rs`, the `[features]` tables of `ep9_platform`, `ep9_vm`, `ep9_cli` and `ep9_unit`, and `reserve_code_range_at` under `cfg(any(feature = "jit", test))`. R1, R2: two existing rows are rewrites by README §3.3 (kind cells set, one clarify row; Codex A2). R3: bb5967e changed a Rules example with no row (row added). Note: check 5 passes today because those crates do not exist yet. The guard-13 row is not there yet (the fixer adds it, M2-P2-GUARD13-H27) | ccaf5df |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | A2 (medium), H27 part: P2-ACCEPT-1 found that the 2026-10-21 "As built in M0 wk4" and "Check 12" rows are rewrites but appended clarify rows, so the original kind cells still read fill and clarify, and a count of rewrite lines finds neither. Reproduced by the P2 verifier: real, category (b). Fixed in 0c68c7d (M2-P2-A2, M2-P2-RF-A2-EDIT): kind cells edited in place to `rewrite (kind set 2026-10-22; was …)`, one clarify row | decisions-log M2-P2-REVIEW, M2-P2-A1, M2-P2-A2; Codex text outside the repo: scratchpad m0m2/p2-codex-bcrhkdqte.md |
| 3 Verifier | P3-ACCEPT-2 (wrote none of the rows it checks) | Checked at e719ffa. A2 fix: the kind cells of H27:110 and H27:112 read `rewrite (kind set 2026-10-22; was fill/clarify)`, and the clarify row H27:113 names both; README §3.3 makes both rewrites (a Rules check text changed; check 12 now rejects what it accepted): agree. The bb5967e clarify row (H27:114): `Reservation::reserve_code_range_at` is under `cfg(any(feature = "jit", test))`, `crates/ep9_platform/src/va.rs:135`: true. The P2 fixer's guard-13 row (H27:115): kind fill agreed (a new Rules row, no existing check changes, as the check 11 and 12 fill); its Check cell was stale (V1, M2-P3-H27-65): the I7 guard (`.github/scripts/dep-guards.sh:215-277`) has 9 must-flag samples, 3 must-not lines, 2 tests of the Cargo.toml rule and also scans `.cargo/config`; cell rewritten (kind rewrite). Part 4 again at e719ffa: `bash .github/scripts/dep-guards.sh` 29 PASS, 0 FAIL (check 13: 3 PASS); `cargo xtask feature-matrix` 9 of 9 PASS. Change-log kinds: agree | c68971a; local run, mac a64, e719ffa; logs outside the repo: scratchpad m0m2/accept2/logs/depguards.log, featurematrix.log |
| 4 Data | from the "Validated by" line | `bash .github/scripts/dep-guards.sh`: 26 PASS, 0 FAIL (checks 5 and 9-12 with their controls). `cargo xtask feature-matrix`: 9 of 9 PASS. SP-JIT-mem: PASS on 4 targets ([M-JIT]). Missing: test262 per mode (M1+) | local run, mac a64, 383a317; logs outside the repo: scratchpad m0m2/accept1/h27-dep-guards.log, h27-feature-matrix.log |
