# H27: Compilers as Cargo features

| Field | Value |
|---|---|
| Status | Draft (2026-10-21). P1-AOTU adds the as-built details of week 4 |
| Fix by | M0 (wk4) |
| Freeze point | M1 exit |
| Origin | v1#25; plan status E (extended: `aot`) |
| Crates | `ep9_cli` (default `jit`), `ep9_vm` (`jit`), `ep9_platform` (`jit`, `unwind`), `ep9_unit` (`aot`, empty in M0); later `ep9_masm`, `ep9_baseline`, `ep9_opt`, `ep9_regexp` (RX2 under `jit`), `ep9_aot`, `ep9c` |
| Owner (plan role) | JIT/platform lead (`jit`); AOT senior (`aot`) |
| Validated by | SP-JIT-mem (passed on 4 targets, 2026-10-21); dep-guards checks 5, 9 and 10; `cargo xtask feature-matrix` as a build-test step on 4 runners (M0 wk4) |
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

## Rules

| Rule | Check | From |
|---|---|---|
| The jitless `ep9` links no `ep9_masm`, `ep9_baseline` or `ep9_opt` | dep-guards check 5 (`.github/scripts/dep-guards.sh:57-62`) | M0 wk1 |
| The jitless `ep9` has no `ep9_platform` feature `jit` | dep-guards check 9 (`cargo tree -p ep9_cli --no-default-features -e features`) | M0 wk4 |
| No `ep9_unit` feature `aot` in the `ep9_cli` graph, default or jitless | dep-guards check 10 | M0 wk4 |
| Each build is built, linted (`-D warnings`) and tested per package, never with `--workspace` | `cargo xtask feature-matrix`; CI build-test step on 4 runners | M0 wk4 |
| Code under `jit` compiles out without the feature | `#[cfg(any(feature = "jit", test))]` (for example `Cage::reserve_jit`, `crates/ep9_platform/src/va.rs:385`) | M0 wk3 |
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
