# ep9 research reports

- These 18 reports are the evidence base for the plan. The plan cites them by tag: [R01] = `01-*.md`, and so on.
- Reports 01-09 come from the JIT research round. Reports 10-17 come from the AOT round. Report 18 comes from the sound-regions round. It was revised in place after an adversarial review, so it has no separate refutation report.
- Two rules settle conflicts: [R17] corrects [R16], and [R09] arbitrates among [R01]-[R08].
- Paths inside the reports point into a session scratchpad. They were session-local and no longer exist. Cite the tag, not the path. Since 2026-10-21 they read `scratchpad/...`, `~/...` (the home folder of the research host) or a repo-relative path, without the absolute session prefix. A file that was copied into [artifacts/](artifacts/README.md) is linked there.
- The plan starts at [../PLAN.md](../PLAN.md).
- Reusable harnesses are kept in [artifacts/](artifacts/README.md): the tsrs type oracle, the tsrs/oxc test262 scorer and, since 2026-10-21, the R18 sound-regions scripts and cited results ([artifacts/sound-regions/](artifacts/sound-regions/README.md)) and the inputs of the 2026-10-20 decision round ([artifacts/decide/](artifacts/decide/README.md)).

## Run context

| Item | Value |
|---|---|
| Date | 2026-10-20 |
| Machine | Apple M5 Max, 18 cores, macOS 27 |
| tsrs pin | f527e984 (`f527e984d6e4d7141b387361bf74180acd6949d6`, `third_party/tsrs.commit`) |
| test262 pin | d53ebc8 (2026-10-13) |
| oxc | 0.155.0 (crates.io) |
| Toolchain | rustc 1.99.1 |
| Reference engines | node 24.22 (V8 13.6), bun 1.4.3 (JSC), QuickJS 2026-06-04 |
| Noise | The machine was shared: load average 10-37. Single cells can be off by 10-20% |

## Index

| Tag | File | Topic | Main result |
|---|---|---|---|
| [R01] | [01-tsrs-architecture.md](01-tsrs-architecture.md) | tsrs parser internals | Strong syntax; early errors live in the checker; no goal or strict state; fixed 32 GiB reservation |
| [R02] | [02-tsrs-conformance.md](02-tsrs-conformance.md) | tsrs and oxc on test262 | tsrs: 99.90% positives, 26.78% negatives. oxc + semantic: 99.98% and 100% |
| [R03] | [03-parser-perf-and-features.md](03-parser-perf-and-features.md) | Parser speed and engine features | oxc 327 MB/s vs tsrs 212 MB/s summed; neither parser can preparse or re-parse one function |
| [R04] | [04-parser-alternatives.md](04-parser-alternatives.md) | Six front ends compared | Only oxc + semantic catches 100% of parse-phase early errors (4,653/4,653) |
| [R05] | [05-gc-design.md](05-gc-design.md) | GC design | JSC-like hybrid: non-moving, CSS, sticky-mark generations, concurrent SATB; MMTk not the v1 core. The SATB slow path is superseded by [H11](../adr/H11-barrier-and-atomics.md) (retreating wavefront, decided 2026-10-20) |
| [R06] | [06-execution-tiers.md](06-execution-tiers.md) | Tiers and dispatch | Mid tier = about 54% of V8 full on Octane and equal to V8 on the TS workload |
| [R07] | [07-rust-engine-landscape.md](07-rust-engine-landscape.md) | Rust engines and lessons | test262.fyi rates; late perf choices caused the biggest regrets |
| [R08] | [08-spec-subsystems.md](08-spec-subsystems.md) | Spec subsystems | Build vs reuse: temporal_rs, ICU4X, dragonbox; regress runs 0.8-5x slower than V8's regexp interpreter |
| [R09] | [09-critic.md](09-critic.md) | Critic of R01-R08 | Resolves number conflicts; adds rooting, snapshot, Windows and panic topics |
| [R10] | [10-js-aot-precedents.md](10-js-aot-precedents.md) | 18 JS AOT precedents | No precedent beats a top-tier JIT across the board; typed AOT wins only on numeric fixed-layout code (nbody) |
| [R11] | [11-dual-mode-vms.md](11-dual-mode-vms.md) | Dart, Graal, .NET, ART, Leyden, Hermes | One optimizer per VM; modes differ in fact sources and in deopt |
| [R12] | [12-ts-type-soundness.md](12-ts-type-soundness.md) | Can TS types be trusted? | 24 crafted hole classes pass strict checks; types prove int32 for only 1-9% of expressions |
| [R13] | [13-tsrs-type-oracle.md](13-tsrs-type-oracle.md) | tsrs as an offline type oracle | Prototype joins 99.94% of 556,724 oxc nodes; 0.87 s for 18 MB with 16 checkers |
| [R14] | [14-dual-mode-architecture.md](14-dual-mode-architecture.md) | Dual-mode architecture | Untyped AOT lands near baseline-JIT speed; deopt metadata, relocations, iOS, frame home |
| [R15] | [15-aot-perf-reality.md](15-aot-perf-reality.md) | Measured AOT vs JIT | Static Hermes typed AOT did not beat V8's top tier on any measured workload (scriptc's nbody win is in [R10]); warm-up data |
| [R16] | [16-aot-synthesis.md](16-aot-synthesis.md) | First AOT synthesis | Dual mode is buildable; no general "AOT beats V8". Corrected by [R17] |
| [R17] | [17-aot-refutation.md](17-aot-refutation.md) | Adversarial review of [R16] | Keeps the architecture and the A4 kill; corrects scriptc, startup, memory and span-join claims |
| [R18] | [18-sound-regions.md](18-sound-regions.md) | Automatic sound regions (no opt-in subset) | Feasible and test262-legal with checks and fuses; real-app hot coverage 0-11% (vscode diff 24-39%); jitless gain est. +0-0.8% on tsc, hono, date-fns; keep D17 = Defer and add a <= 1 PM coverage pre-check. Decided 2026-10-20: D17 rule with option (d), SP-RegionCoverage in A1h ([decisions-log §5](../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction)) |

## Notes for readers

- **Decision numbers.** [R14] numbers its own proposals D1-D10, and [R16] numbers D1-D14. These are not the plan's user decisions D1-D21. The plan writes them as R16-Dn.
- **Crafted suites.** The 24 hole classes [R12] and 13 oracle holes [R13] are crafted. They show that holes exist, not how often they occur.
- **Typed AWFY port.** The typed AWFY port in [R13] and [R18] was written for ep9. It is a best case by construction, not third-party user code.
- **[R18] inputs.** The semantics and hot studies behind [R18] saved no report file. [R18] inlines the probe table, the fuse list and the data it uses. Its fuse IDs F1-F8 are local to [R18].
- **Prototype name.** [R13] calls its prototype `ep9_typecheck`. The plan names the tool `ep9-typecheck` in `tools/ep9-typecheck`.
- **Plan v1 probes.** The plan tags plan v1 local probes as [P1]. They were session-local too. The named M0 spike checks each one again.
