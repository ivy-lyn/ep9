# ep9 research reports

- These 22 reports are the evidence base for the plan. The plan cites them by tag: [R01] = `01-*.md`, and so on.
- Reports 01-09 come from the JIT research round. Reports 10-17 come from the AOT round. Report 18 comes from the sound-regions round. It was revised in place after an adversarial review, so it has no separate refutation report. Report 19 comes from the 2026-11-18 VM-lessons round: .NET, Dart, Kotlin. Its findings were checked by adversarial verifiers and its synthesis by a critic, and the fixes are applied in place. Report 20 comes from the 2026-11-18 best-fit data-structure round, with the oxc study. Two adversarial verifiers (security, gain) checked its synthesis; their verdicts are applied in place, and its refuted items are listed in the report. Report 21 comes from the 2026-11-19 month-10 plan research on the x86 TS time gap; a checker read the draft and its findings are fixed in place. Its section 9 addendum (2026-11-21) adds the month-10 reads; a checker read it, and its findings are fixed in place. Report 22 comes from the 2026-11-21 "why M2 is stuck" round: 5 angle reports, 2 adversarial verifiers and a critic, then 2 experiments (X1, X2) and an adversarial verifier of the experiments; the corrections are applied in place.
- Two rules settle conflicts: [R17] corrects [R16], and [R09] arbitrates among [R01]-[R08].
- Paths inside the reports point into a session scratchpad. They were session-local and no longer exist. Cite the tag, not the path. Since 2026-10-21 they read `scratchpad/...`, `~/...` (the home folder of the research host) or a repo-relative path, without the absolute session prefix. A file that was copied into [artifacts/](artifacts/README.md) is linked there.
- The plan starts at [../PLAN.md](../PLAN.md).
- Reusable harnesses are kept in [artifacts/](artifacts/README.md): the tsrs type oracle, the tsrs/oxc test262 scorer and, since 2026-10-21, the R18 sound-regions scripts and cited results ([artifacts/sound-regions/](artifacts/sound-regions/README.md)) and the inputs of the 2026-10-20 decision round ([artifacts/decide/](artifacts/decide/README.md)), and since 2026-11-18 the raw findings and verify verdicts of [R19] ([artifacts/vm-lessons/](artifacts/vm-lessons/README.md)), and since 2026-11-19 the measured numbers of [R21] ([artifacts/x86-ts-time/](artifacts/x86-ts-time/README.md)), and since 2026-11-21 the numbers of [R22] ([artifacts/m2-stuck/](artifacts/m2-stuck/README.md)).

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
| [R19] | [19-vm-aot-jit-lessons.md](19-vm-aot-jit-lessons.md) | AOT/JIT lessons from .NET (Roslyn, RyuJIT, ReadyToRun, NativeAOT), Dart and Kotlin (K2, Kotlin/Native) with HotSpot Leyden | 115 findings (88 confirmed, 27 corrected, 0 refuted); 12 lessons. ADOPT: a JIT-mode per-function give-up counter, a unit key from one registry, stress knobs per transition, a debug trap on PROVEN facts, a mode-branch count. CONSIDER: .NET's reason to re-tier hot R2R code through an optimized instrumented tier, not Tier0. 15 follow-ups LRN-01 to LRN-15, text only before the M2 gate |
| [R20] | [20-bestfit-data-structures.md](20-bestfit-data-structures.md) | Best-fit data structures for heap containers; the oxc optimization study (oxc-project/oxc@3464a0d18) | A faster HashMap cannot pass the 0.2% kill line: all std SipHash cost is compile time, at most 0.174% (earley-boyer). The real hash cost is ep9 re-interning static Rust text: KEEP B1 (static atom text gives its root; deltablue est. -1.3..-1.4%) and B2 (intern guard split; t0-gen est. -2.6..-2.8%, a call shape, handed back). Refuted: a flat Fx scope map and the oxc ident hash on hostile names (HashDoS). A hasher rule for ep9 and 5 hash weaknesses already in the tree, handed back |
| [R21] | [21-m2-x86-ts-time.md](21-m2-x86-ts-time.md) | x86 TS time for the M2 gate: the need per cell, the Ir-to-time factor, real counters against cachegrind, the V8 lazy accumulator and Star lookahead, the month-10 lever sum | Every x86 TS cell needs ×1.11-1.15 (linux-x64 jit 0.902 / 0.914, jitless 0.888 / 0.893). 1% of x64 TS Ir bought 0.31-0.46% of x86 time in month 9 (Mac a64 0.49-0.69); real branch misses are 12x below the cachegrind count; 64 B handler alignment gave jitless TS 0.975 at equal instructions. 69.87% of the static TS `Ldar` sites are dead loads that V8 never emits. With the measured factors the month-10 levers give a middle of ×1.046 (jit) / ×1.060 (jitless), so month 11 needs ×1.064 / ×1.080 more: only PGO or REG-SLOT-ENCODE can give it; §9 (2026-11-21): the x86 TS gap is cycles per instruction (equal instructions to node --jitless, IPC 2.462 against 2.688), the mid Path B moved the TS cells only ×1.013-1.038 so the low ends need about ×1.10 more, and the REG-SLOT-ENCODE probe cut x64 TS Ir by 7.9-8.0% with its x86 time not read |
| [R22] | [22-m2-stuck-causes.md](22-m2-stuck-causes.md) | Why M2 is stuck: the ranked causes, the lever ledger, the gate and process costs, and the experiments X1 (bytes against placement) and X2 (dynamic counts) | On x86, ep9 T0 runs 9.4-10.6% more cycles than node --jitless at equal instructions (m9-p5, 1 box) (C1, about 60% of the need); at LEVER-13 the jit binary became 5.8% slower than the jitless binary in the same rounds, with a host-dependent sign (C2, about 40%); X1: the handler bytes are equal in both builds, so C2 is not codegen and layout is the lead hypothesis. The levers kept the Ir meter after the instruction gap closed: x64 TS Ir -9.6%, box-neutral time -0.7% (jit) / -8.3% (jitless). The cycle gap sits in frozen parts (named load +0.89 G, dead statement Ldar +0.73 G, Wide +0.59 G); X2: 152.0 M dead statement-position Ldar runs (KEEP, est. 2.2% Ir), a slot order leaves 24.5 M prefixed ops (DROP). 14 of 17 open cells are open by procedure. Orchestrator decisions D1-D7 (keep the gate; time A/B before merge; open the frozen parts under G8) |

## Notes for readers

- **Decision numbers.** [R14] numbers its own proposals D1-D10, and [R16] numbers D1-D14. These are not the plan's user decisions D1-D21. The plan writes them as R16-Dn.
- **Crafted suites.** The 24 hole classes [R12] and 13 oracle holes [R13] are crafted. They show that holes exist, not how often they occur.
- **Typed AWFY port.** The typed AWFY port in [R13] and [R18] was written for ep9. It is a best case by construction, not third-party user code.
- **[R18] inputs.** The semantics and hot studies behind [R18] saved no report file. [R18] inlines the probe table, the fuse list and the data it uses. Its fuse IDs F1-F8 are local to [R18].
- **Prototype name.** [R13] calls its prototype `ep9_typecheck`. The plan names the tool `ep9-typecheck` in `tools/ep9-typecheck`.
- **Plan v1 probes.** The plan tags plan v1 local probes as [P1]. They were session-local too. The named M0 spike checks each one again.
