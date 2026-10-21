# ep9 decisions log

- This log records decisions D1-D21 and the user statements behind them. Since 2026-10-20 it also records the decisions that the agent team made under the user's delegation (§5). The options, decisions and re-check rules live in [plan/11-decisions.md](plan/11-decisions.md).
- Hard architecture decisions H1-H37 are not logged here. They live as ADRs in [adr/](adr/README.md).
- Rule: append only. A changed decision gets a new dated entry; the old entry stays.
- Each entry names its source. Only the user's own words decide, or the agent team under the user's delegation of 2026-10-20 (§5). Text from tools or workflow scripts is marked as such.

## 1. Before the plan

The user fixed 8 items before the plan was written: product, full ECMAScript scope, speed priority, the tsrs question, GC preference, `unsafe` allowed, out-of-scope items and the `third_party/` dependency style. See [11-decisions §1](plan/11-decisions.md#1-fixed-by-the-user).

## 2. 2026-10-20: M0 starts

**User statement (verbatim):** "go, let's start with your ultracode"

**Reading.** "go" starts M0 on the plan defaults. So D1, D2, D7, D8, D12 and D14 take the default that 11-decisions states. The workflow task for this run gave that reading; the user did not name each D item.

| ID | Question | Decision = plan default, as 11-decisions states it | Plan deadline | Affects |
|---|---|---|---|---|
| D1 | Final parser path | **E**. Option E: "C now, local oxc patches (≤ 3), D only on a P-gate, tsrs out of process" (C = oxc pinned) | M0 wk3 | H1, H26; P1-P5; M0-M1 |
| D2 | v1 platforms | "macOS arm64, Linux x64/arm64, Windows x64, all tier-1 from week 1. `ep9 check` and `ep9c build --hints` are unix-only until SP-tsrs-win passes. Windows arm64 after v1. iOS arm64 only if D16 = yes" | M0 wk1 | H28, G10; A3 |
| D7 | Proper tail calls | "Accept the gap: 35 tests, 0.07% [R09]. `TailCall` opcode reserved; no Rust recursion. The H14 ADR (M0 wk3) shows that frame reuse fits the layout, AOT frames included" | M0 wk3 | H14; M5, M8 |
| D8 | Toolchain | "Stable 1.99.1. The nightly build is opt-in" | M0 wk1 | H15, H26 |
| D12 | Upstream contacts | "The user's oxc contact, if confirmed. No tsrs request (its port rule forbids it). The oracle uses the tsrs public API (≤ 2 local patches, est.) [R13]" | M0 wk1 | P1; A1h; RK5, RK38 |
| D14 | Worker heap model | "Isolate per thread" | M0 wk3 | H11, H22, H23 |

Notes:
- **D1 in the repo.** The root `Cargo.toml` pins 6 oxc crates at `=0.155.0` from crates.io (skeleton, 2026-10-20; corrected 2026-10-21, the first text said 7).
- **D12 is not complete.** The default says "the user's oxc contact, if confirmed". No contact is confirmed in this log. Until the user names one, the oxc track runs without a contact; local patches carry more weight and P1 may trip earlier (11-decisions).
- **D7 and D14** had deadline M0 wk3. They are now fixed at M0 start. A later change needs an ADR amendment of the rows they affect (D14: H11, H22, H23).

### D3: team

| Item | Value |
|---|---|
| Plan default | "6 seniors (lead/front end, GC, VM, JIT/platform, runtime, AOT) + agents. v1 at about 57-100 months" |
| Recorded on 2026-10-20 | AI agents do the work now, not the 6-senior team. Agents fill the plan roles |
| Source | The workflow task text for this run. It matches the user's "let's start with your ultracode", but the user did not state a team size. **The user should confirm.** |
| Effect on estimates | PM figures are "senior human person-months" (09-milestones). They stay as a size of work. Calendar months = PM ÷ FTE does not apply to an agent team, so the plan's calendar estimates (for example v1 in about 57-100 months) do not apply directly |
| Not in the D3 options | The plan offers 2-3, 5, 6, 9 or 13 seniors plus agents. An agent-only team is a new answer |

Open follow-ups from D3 (for the user; not decided here):
- Human review. The plan puts human ownership on contract crates (CODEOWNERS) and an adversarial review on every `unsafe` change ([10-quality §6](plan/10-quality.md#6-agent-workflow)). With an agent team, the user is the only human. Who reviews? See ADR process question P-4 in [adr/README.md](adr/README.md#4-ownership-codeowners-and-the-adr-lint).
- Re-plan point. The plan says to check each "(est.)" number at its first milestone review (09-milestones). M0 exit is the first point to compare agent throughput with the PM estimates.

## 3. Open decisions

Each stays open. The plan builds on the default until the deadline ([11-decisions §3](plan/11-decisions.md#3-decisions-all-decided-2026-10-20)). Ordered by deadline.

| ID | Question | Default the plan builds on | Deadline | Re-opens at |
|---|---|---|---|---|
| D21 | Interpreter frame home | (a) VM register stack + native control stack (Hermes model), with the canonical frame layout kept so M5c can switch later | M0 month 2 (Spike B) | - |
| D11 | JIT-free mode required? | Yes: AOT-jitless mode ships in v1. T0 gates are also measured in the jitless build | M0 exit | A3 gate; M8 AOT ship review |
| D15 | AOT product claim; v1 timing if A4 trails M7 | (a) now; (b) as the gated research stage A5 = M10; (c) rejected (N10). Timing (t1) | M0 exit; timing at M7 exit | M8 AOT ship review (see §4) |
| D6 | Untrusted-code sandbox | None in v1. The `ExternalRef` rule applies from M0. Units are trusted code, like native addons | M1a | - |
| D16 | iOS / jitless AOT near-term | No: no iOS before v1; A3 gate at M8 exit | M1 exit | K-A3 pre-check (M5 exit); A3 gate |
| D18 | TS-hint investment | (b) cheap oracle now (A1h), consumers only in A4, kill by K-A4H | M1 exit | SP-HintPrecision (off-ramp); M8 AOT ship review |
| D9 | C++ allowed? | Pure Rust: no irregexp FFI, no LLVM AOT backend. Cranelift stays the option for T3 | M2 start | - |
| D13 | Baseline tier | Build T1. It is also the A3 baseline-AOT emitter | M2 exit | - |
| D10 | Intl data | Full data in the `ep9` binary. A lean feature for embedders | M3 start | - |
| D5 | WebAssembly | wasmtime JS API after v1. Signal and VA coexistence rules apply now. The M6 app gate uses an app with no `.wasm` | before M4 | - |
| D4 | Node compatibility depth | ESM + CJS + V8 stack API in M4. Legacy decorators through oxc_transformer, only in files that use them (M4); those files get no hints in v1. Node-API in M6 | M4 start | - |
| D20 | Class-field semantics | (b) follow tsconfig (`useDefineForClassFields`, `experimentalDecorators`) when present, else [[Define]] | M4 start; header fields from M0 wk3 | - |
| D19 | AOT compile scope | (b) PGO hot set for hybrid; (c) baseline-AOT for all reachable functions + optimized-AOT for the hot set for jitless | M5 start | - |
| D17 | Sound typed dialect | Defer to M7 exit, after SP-TypedBound. If built: (c), per module, opt-in, outside test262 mode | M7 exit | - |

## 4. D15 framing note (choose at M0 exit)

**What the user chooses.** Two things, at two times ([11-decisions D15](plan/11-decisions.md#32-d15-d21-new-for-dual-mode), [05-aot §1](plan/05-aot.md#1-short-answer)):
1. At M0 exit: the product claim for AOT.
2. At M7 exit: the v1 timing, if A4 trails M7.

| Option | Claim | What it means for the plan | Gate that backs it |
|---|---|---|---|
| (a) default now | AOT = startup and warm-up, jitless speed, predictable latency, no JIT compile CPU, possibly smaller total shipped size. The JIT owns general peak | Plan as written. "Predictable latency" is claimed only where the SHORT-RUN latency metric shows it | FV1 (A3; K-A3), FV2 (A4; K-A4P), G13 (A1), G15 report |
| (b) default as A5 = M10 | (a) + "beat V8 at peak on opt-in sound typed numeric class code" | As a gated research stage after M7: needs D17 = yes and a T3-class backend. As a v1 goal: A5 moves before M8 and competes with M7 for staff | FV3 (≥ 1.1x V8 full on ≥ 2 of 4 TYPED-KERNELS and ≥ 1.3x A4 hinted); K-A5pre, K-A5 |
| (c) rejected (N10) | "AOT with TS types beats V8 in general" | No measured system supports it. The architecture would chase a peak gain that guard-removal studies put at 2-8% | none |

| Timing | Meaning | Cost |
|---|---|---|
| (t1) default | v1 waits for the A4 gates | v1 slips 0-6 months (est.) if A4 trails M7 |
| (t2) | v1 ships JIT mode, A1 and the jitless top tier on M7 timing; A4 follows in v1.x | Optimized-AOT, hybrid mode and HINTED facts wait for v1.x |

**Evidence the user has at M0 exit** (no FV gate is measured by then; [11-decisions §4](plan/11-decisions.md#4-evidence-for-d15-d21)):
- No measured JS AOT system beats V8 on general code. Static Hermes typed native is 1.5-3.2x slower than V8 full (geomean about 2.1x) [R15]. Its gap is implementation debt, not a ceiling [R15][R17].
- scriptc ran nbody about 1.15x faster than V8 at peak, but it refuses eval and changes JS semantics [R10][R17].
- Graal PGO native-image beat its own JIT (0.89-0.91 vs 0.92 of C2), with a profile trained on the same benchmarks [R11][R17].
- Hello-world startup and RSS gaps are runtime bootstrap effects, not AOT effects [R17].
- New in M0: the SP-Unit result (unit blob loads, calls, unwinds, GC-scans and falls back on 4 targets).

**What depends on the answer** ([11-decisions §2](plan/11-decisions.md#2-how-the-decisions-depend-on-each-other)): D15 feeds D17 (sound dialect, M10, FV3, K-A5) and D18 (hint investment, A1h, A4, K-A4H).

**Data comes back later.** M7 exit: SP-TypedBound (K-A5pre) and A4 progress against T2 → D17 and D15 timing. M8 AOT ship review: K-A4J, K-A4P, K-A4H, the AOT-vs-own-JIT report, latency and the G15 report → D11, D15, D18. No AOT part leaves v1 until the user confirms there.

**Questions for the user at M0 exit:**
1. Keep claim (a) for v1? Or make (b) a v1 goal, with its staff cost against M7?
2. Confirm that claim (c) stays rejected.
3. Any early view on timing (t1) vs (t2)? The plan asks for it only at M7 exit.

Evidence tags: [R09](research/09-critic.md), [R10](research/10-js-aot-precedents.md), [R11](research/11-dual-mode-vms.md), [R13](research/13-tsrs-type-oracle.md), [R15](research/15-aot-perf-reality.md), [R17](research/17-aot-refutation.md).

## 5. 2026-10-20: decisions made by the agent team under the user's instruction

**User statement (verbatim, relayed by the workflow harness):** "Don't ask me questions. Deepresearch and descide yourself"

**Reading.** The user hands every open item to the agent team: D1-D21, the R18 plan questions, the open ADR questions of H06, H11, H35 and H36, and the governance items. The agents decide each item with evidence. They prefer the reversible option when two options are close, and they make nothing public. The owner reviews at will and can override any entry. The D3 wording ("AI agents led by an orchestrator, with the human owner reviewing at will") comes from the workflow task text, not from the user's words.

**How the rows were made.** Five agents (gc-barrier, math, tech-gaps, governance, d-sweep) proposed decisions. A critic agent challenged each one. The apply agent made the final call: a "change" whose evidence held when re-checked → the better decision; a "keep" → the decision as proposed; every critic point is in the last column. Scratch evidence paths are under the session scratchpad `work/decide/<agent>/` (session-local).

**What this resolves.** §2 "The user should confirm" (D3) and the D12 note: decided below. §3 "Each stays open": every row is decided below. §4 questions for M0 exit: claim (a) kept, (c) stays rejected, timing (t1) kept.

Kinds: **now** = final until its re-check point or an owner override. **rule** = a written rule picks the outcome from gate data at the named point; nobody is asked a question. Confidence: [high] / [medium] / [low].

### 5.1 D1-D21

| ID | Decision | Kind | Conf. | Evidence | Re-check / reopen rule | Critic note |
|---|---|---|---|---|---|---|
| D1 | E: oxc `=0.155.0`, ≤ 3 local patches; own parser only if a P-gate fails; tsrs only out of process | now | high | [11-decisions §3.1](plan/11-decisions.md#31-d1-d14-from-plan-v1); week-1 parse data | A P-gate fails (P1, P2, P4 at M2 exit; P3, P5 at M4 exit) | keep: same as the default |
| D2 | macOS arm64, Linux x64/arm64, Windows x64 tier-1. A Windows gate counts only after a run on Windows. `ep9 check` unix-only until SP-tsrs-win | now | medium | 11-decisions §3.1; no Windows run yet | SP-tsrs-win; a Windows-only fix > 3 PM before M2 exit | keep: stricter, honest rule |
| D3 | AI agents led by an orchestrator fill the six roles. Owner reviews at will. Required reviews follow ADR-P1 (reviewer agent + Codex adversarial review). PM = size; no calendar | now | high | [09-milestones](plan/09-milestones.md) header; §2 above | M0 exit: PM per week vs plan > 2x off → re-plan PM ranges; owner override | keep; wording changed to name the Codex review (taken) |
| D4 | ESM + CJS + V8 stack API (M4); Node-API (M6); legacy decorators without hints; node/test exclusions only with a reason class | now | medium | [07-runtime §6](plan/07-runtime.md) | M4 start: an entry without a reason class, or exclusions > 10% (est.) | keep |
| D5 | wasmtime JS API after v1; JetStream `--no-prefetch` | rule | medium | [measurements](measurements/2026-10-20-jetstream-maxopt.md) | Before M4: ≥ 2 M6 app candidates need Wasm, or ≥ 5% (est.) of npm top-1,000 ship `.wasm` → Wasm in v1 | keep |
| D6 | No sandbox in v1; `ExternalRef` rule; units are trusted code | now | high | 11-decisions §3.1 | Owner request before M1a | keep |
| D7 | Accept the PTC gap; `TailCall` reserved | now | high | §2 above | H14 ADR finds frame reuse does not fit | keep |
| D8 | Stable 1.99.1; nightly `become` opt-in | now | high | §2 above | A stable bump breaks the 0-non-tail-exit check | keep |
| D9 | Pure Rust; C only in non-shipping references; an RX1 miss becomes RegExp debt | now | medium | 11-decisions §3.1 | RX1 < 0.64x node --jitless at M5 exit and no Rust fix within 4 PM (est.) | keep |
| D10 | Full baked Intl data; lean feature | now | high | 11-decisions §3.1 | SIZE-REPORT blocks a named embedding target | keep |
| D11 | Jitless mode ships; gate data picks only its top tier (baseline-AOT, else M5c, else T0) | now | high | 11-decisions §3.1 | Only an owner request removes the mode | keep: safe, T0 is always a valid top tier |
| D12 | No upstream contact; front-end lead agent owns the oxc track; local patches + drafts; only the user posts | now | medium | [12-governance §5](plan/12-governance.md#5-upstream-contact-oxc-and-other-public-projects) | P1 at M2 exit; the owner names a contact or files drafts | keep; shared lines use the UPSTREAM text (taken) |
| D13 | Build T1 (JIT baseline, A3 emitter, M5c generator) | rule | high | 11-decisions §3.1 | M2 exit: build T1 while D11 = yes | keep |
| D14 | Isolate per thread | now | high | §2 above | Only by an H11/H22/H23 amendment | keep |
| D15 | Claim (a); (b) only as gated A5, mechanism-neutral; (c) rejected. Timing (t1): v1 waits for the A4 gates; failing A4 parts leave by K-A4J/K-A4P/K-A4H; (t2) only on owner request | now | high (claim), medium (timing) | [11-decisions §4](plan/11-decisions.md#4-evidence-for-d15-d21) | M8 AOT ship review; owner request for (t2) | **change taken**: the proposed (t2)-by-size rule contradicted D3 (a slip has no fixed cost) and would drop the top goal's AOT parts from v1 |
| D16 | No iOS before v1 | rule | medium | 11-decisions §3.2 | K-A3 pre-check / A3 gate: baseline-AOT ≥ 1.6x T0 and SP-VA passes on iOS with a ≤ 4 GiB cage → iOS in v1.x | keep |
| D17 | (b) rejected. M7-exit rule: (d) VERIFIED regions > (c) opt-in subset > (a) none; K-A4J trip stops (d) | rule | medium | [R18](research/18-sound-regions.md); 11-decisions §3.2 | A1h (K-A5v-cov); M7 exit (K-A5pre, K-A5v-bound); M8 exit (K-A4J) | keep |
| D18 | (b), with the A1h off-ramp to (c) for HINTED consumers | now + rule | medium | [05-aot §5](plan/05-aot.md#5-where-hints-should-pay-hypotheses-unmeasured) | A1h off-ramp; K-A4H at M8 exit | keep |
| D19 | Hybrid: hot set. Jitless: baseline-AOT all + optimized hot set; A3 size switch | rule | medium | [05-aot §9](plan/05-aot.md#9-what-aot-compiles-d19) | A3 SIZE-REPORT > 8x | keep |
| D20 | Follow tsconfig, else [[Define]] | now | high | [01-front-end §9](plan/01-front-end.md#9-running-typescript-changed) | None by gate data | keep |
| D21 | (a) Hermes model; Spike B places JIT frames (≤ 18 dispatches per round trip keeps native-stack JIT frames) | now + rule | medium | 09-milestones §4 | Spike B fails both placements; Spike C cannot copy frames | keep |

### 5.2 R18 plan questions

| ID | Decision | Conf. | Evidence | Reopen rule | Critic note |
|---|---|---|---|---|---|
| R18-1 | (d) is a D17 option. Nothing built before the M7 rule. Zero-cost reservations in H30 (`VERIFIED{region}`) and H32 (failure rows) before M1 exit | medium | [02-architecture §6](plan/02-architecture.md#6-hard-to-change-decisions-h1-h37) H30, H32 | The D17 rule drops (d) | keep |
| R18-2 | SP-RegionCoverage inside A1h (≤ 1 PM; A1h 1-3 PM); K-A5v-cov stays at 15% on the lower of two profiles | medium | [09-milestones §4](plan/09-milestones.md#4-spikes) | Finder under-counts > 2x; D18 → (c) | keep: 15% re-derived (break-even 13.7%) |
| R18-3 | H24 debugger rule: local writes only on T0 frames; binding writes pop the DebuggerBindingWrite fuse; every poll is a lazy-deopt point; CDP probes at M4 exit | medium | 02-architecture H23, H24, H29, H31 | A probe shows a wrong-type read or crash; users need writes into optimized frames | keep: premise correction confirmed (field retyped via `Runtime.evaluate`; locals via `setVariableValue`) |
| R18-4 | K-A5v uses AND: stop only if the real-app part (< 3%) and the kernel part (FV3) both fail; A5v safety stop | medium | [05-aot §13](plan/05-aot.md#13-kill-criteria) | Region-code upkeep > 1 PM/yr with one part passing | keep |
| R18-5 | No general A5v stage; kernels-only A5v only if region code ≥ 1.3x A4 on ≥ 2 of 4 TYPED-KERNELS | medium | 05-aot §13 | SP-RegionBound meets 1.3x | keep |
| R18-6 | Claim (b) wording is mechanism-neutral; "found automatically" only if (d) passes FV3, for those kernels | high | [PLAN §1.1](PLAN.md) item 14 | FV3 and the K-A5v real-app part pass under (d) | keep |
| R18-7 | Copy the R18 sources and scripts into `research/artifacts/sound-regions/`, paths replaced by placeholders | high | [artifacts/sound-regions](research/artifacts/sound-regions/README.md) | Copy grows past 2 MB | keep. Applied 2026-10-21 with sources and scripts only (41 files, 272 KB); result files stayed in the scratchpad (apply rule: no data) |

### 5.3 ADR questions (H06, H11, H35, H36)

| ID | Decision | Conf. | Evidence | Reopen rule | Critic note |
|---|---|---|---|---|---|
| H11-algorithm | JSC Riptide barrier: incremental update with a retreating wavefront, not SATB. One object-granularity barrier for S1-S3; new-value filter; the slow path does the store | medium | [H11](adr/H11-barrier-and-atomics.md); [03-gc §3](plan/03-gc.md#3-barrier-one-fast-path-in-every-tier-and-code-kind) | S2b p99 pause gate fails after scheduler tuning → evaluate object-logging SATB (option B) | **change taken**: the critic's exhaustive model (re-run 2026-10-21: "decided LOST C; fix_rescan ok; fix_push ok") showed a lost object through LOS cards. Added: dirty LOS cards are a fixpoint root; a BLACK LOS host is pushed once; emitter rule; weak-slot clearing at every minor GC; Acquire hand-off; TLAB retire before CSS |
| H11-fastpath | `if is_cell(v) && host.gc_state <= threshold { barrier_slow(host, slot, v) } else { store }`; 4 instructions + cell test on aarch64 | high | H11; SP-Value `ep9_probe_barrier` | Disassembly > 4 + cell test; H11-Q5 inlines; mask or offset changes | keep; emitter rule added (no GC point between check and store; reload after a GC point) |
| H11-slowpath-S1 | Threshold GEN = 1. Store, then BLACK host → GREY + remembered-set push; LOS host → dirty 512-byte card | high | 03-gc §3-§4 | Remembered set dominates minor GC on Splay; card scan > 25% of minor GC | keep; record the S1 call rate into old LOS arrays |
| H11-slowpath-S2 | Threshold MARKING = 0xFF. Store Release, fence SeqCst, reload gc_state; BLACK → GREY + push; LOS host: dirty card, and push once if BLACK; markers load with Acquire | medium | 03-gc §3 | loom finds an uncovered interleaving; slowdown while marking > 10% | **change taken** (same model): card-only path lost an object |
| H11-termination | Stop-the-world fixpoint rescans every root source: CSS after a TLAB retire, VM stack, `Global<T>`, `ScratchRoots`, unit RW cells, embedder roots, mutator mark stack, dirty LOS cards, weak and ephemeron constraints | medium | 03-gc §3; [H10](adr/H10-rooting.md) | Pause gate fails; one rescan > 1 ms | **change taken**: dirty cards and TLAB retire added; card table registered in H10 |
| H11-allocation-colour | Allocate WHITE in every stage; RW cells are roots with no barrier; no weak read barrier; every minor GC and S2 end clears weak slots to unmarked referents | high | 03-gc §3 | Final fixpoint too long; a pointer store outside barriered slots and roots | **change taken**: an S1 weak slot to a dead young object would dangle without clearing |
| H11-elision | E1 non-cell value; E2 fresh, unescaped host with no GC point since allocation; E3 stores into roots. No clustering | medium | 03-gc §3 | Barrier cost > 3% of JIT time on JetStream 3 | keep; emitter rule as in H11-fastpath |
| H11-verifier | Model test (6 scenarios incl. `los`), V1 remembered-set verifier, V2 closure, V3 re-mark, 5 negative controls, loom with 2 markers | high | [03-gc §7](plan/03-gc.md#7-gc-test-matrix-per-mode); [10-quality §2](plan/10-quality.md#2-suites) | An unattributable verifier failure; V3 cost | keep; `los` scenario, 5th control and 2-marker loom added |
| H36-libm | Correctly rounded target; one crate `ep9_math` for all tiers, AOT and folds; no host libm for a JS-visible value | high | [H36](adr/H36-fp-semantics.md); [04 §10](plan/04-execution.md#10-floating-point-rules-h36) | A CR function > 1.5x V8 after tuning; V8 keeps a non-CR result for ≥ 4 releases and code depends on it | keep; ADR states that the d8 column is from one host |
| H36-libm-src | Own vendored pure-Rust port: CORE-MATH algorithms, pxfm 0.1.30 seed normalized to one path. No NOTICE file; provenance in `THIRD-PARTY-LICENSE`; diff against CORE-MATH C before vendoring | medium | H36; [12-governance §1.3](plan/12-governance.md#13-duties-for-code-that-ep9-plans-to-copy-or-port) | Normalization fails the CR oracle or passes 1.5 PM → port CORE-MATH C directly | **change taken**: a NOTICE file binds Apache redistributors (LICENSE decision: no NOTICE); pxfm headers claim sole BSD-3 copyright (0 hits for CORE-MATH or LLVM in its source) |
| H36-pow | `pow_js`: JS special cases, CR-equal fast paths (2, 0.5, 1, -1), else CR pow | high | H36 | CR pow > 1.5x host pow on a pow-bound gate | keep |
| H36-fmod | Exact pure-Rust `fmod`; never C `fmod` | high | H36 | > 2x slower on a `%`-bound gate | keep; MIT entry in `THIRD-PARTY-LICENSE` |
| H36-lint | Root `clippy.toml` bans + `cargo xtask fp-check` (symbols, FMA, second math crate), 5 targets, allowlist file | high | [10-quality §5](plan/10-quality.md#5-other-ci-checks) | Allowlist > 5 entries; a path escapes; false positives | keep; land `clippy.toml` after the tools/ workflow ends |
| H36-jit-inline | Closed inline list of exact operations; all else calls `ep9_math` | high | H36 | Math call overhead > 5% in T2/A4 → same algorithm only, with a bit-equality test | keep |
| H36-test | Golden bits and 1M-input hashes every PR on 4 targets and x86-64 v1/v3; nightly mpmath oracle and d8 ULP check | high | H36 | Non-CR result unfixable in a week; CI > 2 min per target | keep |
| H36-deps | No dependency computes a JS-visible number with the host libm; oxc_semantic enum values are not used | high | H36; [01-front-end §9](plan/01-front-end.md#9-running-typescript-changed) | ep9 needs those values at run time | keep |
| H36-fpenv | Default FP environment; debug asserts at Isolate entry | medium | [07-runtime §5](plan/07-runtime.md#5-api-boundary-c12-unchanged) | An embedder needs FTZ | keep |
| H36-toint32 | `ep9_value::to_int32`; FJCVTZS when JSCVT is present | medium | H36 Q4 | The helper moves to `ep9_numconv` | keep |
| H36-nan | No extra constraint; H06-Q3 owns the NaN pattern | high | H36 Q5 | H06 skips purification on a path | keep |
| H35-fallback | Three outcomes by field: LOAD ERROR (prefix, hash, signature), SOURCE (build id, bytecode ABI), BYTECODE (native-only versions, CPU, fact schema); `--unit-strict` | high | [H35](adr/H35-aot-unit-format.md) S3-S4; [05-aot §8](plan/05-aot.md#8-aot-unit-format-h35) | Units must survive engine upgrades and SOURCE start-up is too slow → relax step 3 | keep; a `.o` unit's LOAD ERROR is a clean startup error, exit 1 (added) |
| H06-empty | Empty/hole = 0x22; bits stated; no 0 in a heap `Value` slot | high | [H06](adr/H06-value-encoding.md); [06 §1](plan/06-object-model-and-spec.md#1-values-h6-unchanged) | An immediate scheme uses bit 0x20; hole cost > 1% on a v8-v7 kernel | keep; pre-fill rule added (slots are filled before the next GC point; the release visitor skips 0) |

### 5.4 Plan gaps and governance

| ID | Decision | Conf. | Evidence | Reopen rule | Critic note |
|---|---|---|---|---|---|
| JETSTREAM-PROXY | No harness patch. Proxy/Reflect core in M1; fallback patch only if Proxy misses M2 exit | high | [09 M1 scope](plan/09-milestones.md#22-scope-and-exit-criteria); 10-quality §2 | Proxy core slips past M2 exit; a pin bump puts the trap in a timed path | keep |
| DISPATCH-GATE | SP-Dispatch gate on the geomean of the 3 kernels, with per-kernel guards; speed-gate rule | medium | 09-milestones §4 | Geomean passes while a guard fails on x64 or Win64; match beats sibling on a kernel | keep; a64 timings provisional (load 16-21) |
| M2-JETSTREAM-LIST | Proxy, Reflect and WeakRef do not drop a workload; 34 of 63 at the current pin | high | 09 M2 exit 7 | Proxy core leaves M1; an added workload fails outside scope | keep |
| NAMING | One pin per checkout; `<suite>.<target>[.<mode>].txt` | high | [08-repo §1](plan/08-repo.md#1-tree) | The runner merges parse and full results; a monorepo pin | keep |
| PERF-NOISE | Speed-gate rule: classes R/S/M/W/Mem, gate host, load guard, paired rounds, order-statistic verdict, void and provisional labels | medium | [10-quality §3](plan/10-quality.md#3-benchmark-sets) | Paired IQR > 5% under the load guard; no Linux gate host at M1 exit | keep; added: the gate host is an owner action; without it class M Linux gates read "marginal (no gate host)"; callgrind is the class R default on hosted runners |
| LICENSE | `MIT OR Apache-2.0`; no NOTICE file; `THIRD-PARTY-LICENSE`; `cargo deny` allow list | medium | [12-governance §1](plan/12-governance.md#1-licence) | ep9 moves into an MIT-only project; a crate outside the allow list; a copyleft port | keep; duty table widened (any V8 port, pxfm, CORE-MATH, LLVM libc, musl fmod, MPL atomics); GPL-2.0 claim limited to the source |
| REMOTE-CI | Existing private repo, `main`, squash merges, scope job, tiers, concurrency, lean mode, USD 50/month ceiling (low confidence) | medium | [12-governance §2-§3](plan/12-governance.md#3-ci-triggers-and-cost) | Spend far from the ceiling; plan or price change; a macOS-only break reaches main | **change taken**: commit and push only on an explicit user instruction (harness rule); payment method and budget are owner actions; CI may block in month 1 without one |
| ADR-P1 | Review record: reviewer agent + Codex adversarial review + (a)/(b)/(c) sort; the orchestrator never approves its own change | medium | [adr/README §3.1](adr/README.md#31-review-record-p-1) | The user wants to approve Frozen rewrites; rubber-stamping found | keep |
| ADR-P2 | Change kinds clarify, fill, rewrite, reopen by a mechanical test; a dispute means rewrite | high | adr/README §3.3 | Many disputed kinds | keep |
| ADR-P3 | A freeze point per part (Freeze table) | medium | adr/README §3.4 | The plan moves a build milestone | keep |
| ADR-P4 | Advisory CODEOWNERS; no required code-owner review | high | [12-governance §4.3](plan/12-governance.md#43-why-codeowners-is-not-enforced) | Agents get a bot identity; the user gates merges | keep |
| CODEOWNERS | `.github/CODEOWNERS` lists the owner on 12 crates, the decision files and the build files; no catch-all | high | `.github/CODEOWNERS` | A crate is renamed or split; an organization move | keep; check `codeowners/errors` after the first push |
| UPSTREAM | Only the user posts upstream; agents keep drafts in `docs/upstream/` | high | 12-governance §5 | The user names a contact or asks agents to post; ep9 becomes public | keep |
| GITATTRIBUTES | Root `.gitattributes`: LF, `*.patch -text`, binary list | high | [12-governance §6](plan/12-governance.md#6-line-endings) | A tool needs CRLF in a tracked file | keep |

**Order of application (critic CROSS-CONFLICTS).** 1) The H11 text with the critic additions went into 03-gc first. 2) `ci.yml`: only the decided `on:` types and `concurrency:`; the scope job, the nightly schedule, the fp-check job, the cargo-deny step and the `perf` group wait for the tools/ workflow (follow-ups). 3) The UPSTREAM text for the lines that D12 also edits. 4) `THIRD-PARTY-LICENSE` entries instead of a NOTICE file in `ep9_math`.

**Corrections on 2026-10-21.** §2 note "D1 in the repo" said 7 oxc crates; the root `Cargo.toml` pins 6 (`oxc_allocator`, `oxc_ast`, `oxc_diagnostics`, `oxc_parser`, `oxc_semantic`, `oxc_span`). The note is fixed in place.

## 6. 2026-10-21: corrections to earlier entries

**Source.** The hygiene agent of the M0 workflow, from the verify report of the decision round. This entry changes no decision. It records edits that were made in place, and it completes R18-7 as decided.

**Edits made in place on 2026-10-21.** The rule "append only" (top of this log) was broken by four in-place edits. They stay as they are. This table keeps the old text.

| Where | Old text | Text now | Why |
|---|---|---|---|
| Top, bullet 1 | "This log records user decisions D1-D21 and the user statements behind them. The options, defaults and deadlines live in plan/11-decisions.md." | Names the agent-team decisions of §5 | §5 adds decisions made under the user's delegation |
| Top, bullet 4 | "Each entry names its source. Only the user's own words decide." | Adds "or the agent team under the user's delegation of 2026-10-20 (§5)" | Same |
| §2, note "D1 in the repo" | "pins 7 oxc crates" | "pins 6 oxc crates ... corrected 2026-10-21" | Count error. The root `Cargo.toml` pins 6 |
| §3, first line | Link target `#3-open-decisions` | Link target `#3-decisions-all-decided-2026-10-20` | 11-decisions §3 has a new heading, so the old anchor was broken. The text "Each stays open" is history: §5 decides every row of §3 |

**R18-7 is now applied in full.** The §5.2 row says "sources and scripts only (41 files, 272 KB); result files stayed in the scratchpad". On 2026-10-21 the result files were copied too, as the decision says: 68 files, 1,061,250 bytes, in [research/artifacts/sound-regions/results/](research/artifacts/sound-regions/README.md). The folder holds 1,337,334 bytes (limit 2 MB). The largest file has 172,073 bytes (limit 256 KiB). `tsc.json` and `tsc-noinl.json` lost their `per_function` key, as R18-7 says. The decision does not change.

**Scratch evidence.** §5 says "Scratch evidence paths are under the session scratchpad `work/decide/<agent>/` (session-local)". The inputs that later steps need are now in [research/artifacts/decide/](research/artifacts/decide/README.md): `ci-scope.sh` and its tests, the `deny.toml` and `clippy.toml` drafts, the upstream README draft, the two barrier models (H11) and the math tables (H36). A re-run on 2026-10-21 gave the same results. All other scratch evidence stays session-local.

## 7. 2026-10-21: H15 dispatch on x86-64 (SP-Dispatch reopen)

**Source.** The H15 agent of the M0 workflow, under the user's delegation of 2026-10-20 (§5). The round started from the user's message, relayed verbatim by the workflow harness: "What are u watching? CI is already finished and failed". Inputs: CI run [39458801246](https://github.com/Brooooooklyn/ep9/actions/runs/39458801246) (commit `ea31dcb`), the x86-64 diagnosis (branch `diag/dispatch-x64`, 3 runs) and the Windows cgoto.c fix. Data: [measurements/2026-10-21-sp-dispatch-x64.md](measurements/2026-10-21-sp-dispatch-x64.md). Decision record: [adr/H15-interpreter-dispatch.md](adr/H15-interpreter-dispatch.md).

**Trigger.** The SP-Dispatch gate failed on linux-x64: "sibling ≥ 1.2x faster than match (geomean): 0.812", and match beat sibling on every kernel. That is the DISPATCH-GATE reopen rule (§5.4). The plan says a failed spike gate reopens its ADR, so H15 reopened before its first draft.

| ID | Decision | Conf. | Evidence | Reopen rule | Note |
|---|---|---|---|---|---|
| H15-X64 | Release expansion per target: sibling calls on aarch64; the match loop on x86-64 (SysV and Win64). No DSL codegen fix. Debug: match everywhere | medium | x64 match loop has 16 dispatch jumps (LLVM tail duplication), aarch64 has 1; match / sibling time 0.77-0.81 (EPYC 7763), 0.88-0.92 (9V74), 1.03 (9V45), 1.13 (Xeon 8573C and 8370C); Windows 0.83 (7763), 0.97 (9V74); 7 probes, none fixes Zen 3 | The x64 loop check fails on T0, or `match-vs-cgoto` fails on a named x64 model; H15-Q1 at M2 exit. Sibling beating match on a model is H15-Q1 input, not a trigger (the vendor split is known) | **Superseded 2026-10-21 by §10 H15-X64-T0.** The R06 gain of 1.3-1.5x holds on aarch64 only |
| DISPATCH-GATE (amended) | Gate per target. Release expansion ≤ 1.25x cgoto.c (geomean of 3 kernels) and ≤ 1.5x per kernel. aarch64 also: match ≥ 1.2x sibling time (geomean), ≥ 1.0x per kernel. x86-64: sibling vs match is a report, never a gate. x86-64 loop shape (`disasm-check --match-loop`) is a structural gate. M0 exit item 5 on x64: no recorded CPU model fails; marginal and void runs are repeated | medium | 09 §4 row SP-Dispatch (updated) | As H15-X64; aarch64 rule unchanged | **Superseded 2026-10-21 by §10 DISPATCH-GATE (T0).** **change taken**: on x64 the old gate compared threaded code with threaded code. Open before M0 exit (H15-Q3): the cgoto.c compiler differs per target; pin it, then repeat Win64 on EPYC 7763 (loop kernel 1.591x cgoto.c, run void) |
| PERF-NOISE (amended) | Class S on a hosted x64 runner: ≥ 31 rounds, the result names the CPU model and holds for that model only. `sp-dispatch bench` applies the rule: paired ratios per round, median, order-statistic interval, void if a kernel IQR > 5% | medium | Hosted x64 labels gave 5 CPU models in 16 jobs; verdict 0.77-1.13 | A hosted label pins one CPU model, or an x64 gate host exists | **change taken**. The text of 10-quality §3 row S is not edited yet (that file is open in another workflow): add "names its CPU model; holds for that model only" |
| SPEED-CI | Hosted CI: structural gates block. Speed gates never block (`SP_SPEED=report`, 31 rounds): a `::warning` for each speed gate that does not pass, every gate in the job summary, all samples in the artifact `sp-dispatch-gate-<target>` (`bench.json`) | medium | `.github/workflows/ci.yml` spikes job | A gate host joins CI (the `perf` group): its speed gates then block | New. Hosted results stay visible; they do not hide behind a green check |
| D21-UNIT | "18 sibling-call dispatches" (D21, Spike B) means 18 dispatches of the release expansion on the SP-Dispatch loop kernel, on the same host and in the same session. The number 18 does not change | medium | H15 ADR | Spike B design review | **clarify**. Text in 04 §4, 09 §4 and 11 D21 still says "sibling-call"; edit with the next touch of those rows |
| REMOTE-CI (note) | The user allowed commits and pushes to the private repo. The user wrote these messages in the session: "You are free to create repo, keep it private first" (2026-10-20 14:27 UTC) and "Don't stop, I'm going to sleep. Keep working on our plan" (2026-10-20 16:32 UTC). This settles the REMOTE-CI critic note "commit and push only on an explicit user instruction". The repo stays private; no force-push to `main` | high | The user's own messages (session transcript `53ce3b1c`, checked 2026-10-21; §9.3) | The user withdraws the permission or asks for a public repo | Records the instruction that the §5.4 REMOTE-CI row asks for. Corrected 2026-10-21 (§9.2): the first text said the permission was seen only as workflow script text |

**Scratch evidence.** CI logs on GitHub expire. The numbers that the decisions use, with run and job URLs, are in the measurements file. The diag branch `diag/dispatch-x64` keeps the probe code; `diag/h15-gate` ran the new gate on 4 hosted targets before the change goes to `main` (runs [39472609604](https://github.com/Brooooooklyn/ep9/actions/runs/39472609604), [39474039711](https://github.com/Brooooooklyn/ep9/actions/runs/39474039711): every job passed).

## 8. 2026-10-21: BytecodeGen v0 (F1)

**Source.** The bytecodegen agent of the M0 workflow, under the user's delegation of 2026-10-20 (§5). The workflow task text quotes the user: "Don't ask me questions. Deepresearch and descide yourself"; "Keep working on our plan" (workflow script text). Code: [crates/ep9_frontend/src/bytecodegen/](../crates/ep9_frontend/src/bytecodegen/mod.rs). The design table G1-G11 is at the top of `mod.rs`.

```text
source -> parse + semantic -> scope::analyze -> BytecodeGen (G1-G11) -> UnitBuilder::finish -> verify -> VerifiedUnit
```

| ID | Decision | Conf. | Evidence | Reopen rule | Note |
|---|---|---|---|---|---|
| BCG-API | Entry points `compile_script`, `compile_module`, `compile` return `VerifiedUnit` or `CodegenError` (Compile, Unsupported, Limit, Internal). `check_determinism` builds twice on two threads and compares bytes and content hash | high | `bytecodegen/tests.rs::entry_points` | The VM or the tools need another result type | M0 exit 12 helper |
| BCG-SCOPE (G1, G2) | The scope pass decides every binding location. BytecodeGen pushes one context for each scope with `needs_context`, in tree order. Temporaries use register marks (scope based, deterministic) | high | 87 golden files; context check (BCG-CTX-CHECK) | A register optimizer arrives (M2) | No re-allocation in codegen |
| BCG-SCRIPT-CTX (G3) | A script whose top level needs a context gets the script context descriptor as unit scope 1. Later ids move up by one. Scope 0 keeps the global declarations for `DeclareGlobals` | medium | `var-script-this`, `var-direct-eval` goldens | H13 global object design needs another shape | Keeps the verifier rule "outer scopes come first" |
| BCG-FINALLY (G5, G6) | `finally` uses a token register and a value register, then `SwitchOnSmi` (V8 model). `for-of` is a try-finally with a `done` register | high | `ctl-finally-*`, `iter-for-of-*` goldens | Generators (M2) need another shape | Handler tables stay static |
| BCG-TDZ (G8) | A hole store at scope entry only when some reference checks it. Globals and module cells: the VM checks | medium | `var-tdz*` goldens | A TDZ bug in test262 runs (M1) | V8 rule |
| BCG-RUNTIME (G9) | ABI 2: runtime ids 6-18 for global init, direct eval, classes, super, accessors, object spread and `__proto__`. The opcode table does not change | medium | `ep9_bytecode/tests/golden/abi-history.txt` row 2; H02 change log | H13 runtime table (M1) | Bumps `ABI_VERSION` to 2 |
| BCG-POS (G10) | Every instruction that can throw has a position entry (allocations excepted). New builder call `FunctionBuilder::set_throw_position` gives the default | high | 0 missing on 72,147 corpus units | M0 exit 7 needs columns of another kind | Small API addition to ep9_bytecode |
| BCG-UNSUPPORTED (G11) | An unsupported construct fails the whole unit with `CodegenError::Unsupported` and a position. v0 does not support: generators, async, `await`, `for await`, private names, class fields, static blocks, auto-accessors, decorators, `import()`, `import.meta`, `using`, `delete super`, direct eval with spread arguments | high | corpus classes (BCG-CORPUS) | Months 2-3 plan | No silent wrong code |
| BCG-ERR-BOX | Codegen functions return `Result<T, Box<CodegenError>>` | high | Debug stack per AST level of the `call` form: 8,512 B before, 3,776 B after; budget 5,830 B (`stack_tests.rs`) | The budget changes in `stack.rs` | Release worst 1,232 B (budget 3,270 B) |
| BCG-CTX-CHECK | A test-only data-flow check follows context values through each function and checks Create*Context parents, closure contexts, slot depth and slot range | medium | 4 mutation tests catch wrong units that the verifier accepts; 0 errors on 72,147 units | H02-Q2 moves the check into the verifier | Input for H02-Q2 |
| BCG-CORPUS | Corpus run: test262 + `third_party/bench-*`, 54,522 files, 104,375 compiles, each built twice: 0 internal errors, 0 panics, 0 determinism mismatches, 0 missing positions, 0 context errors. A corpus digest (SHA-256 over sorted path, mode and unit hash) compares hosts and targets: `decb0871...8fce76` over 72,147 units on macOS arm64 (debug and release), macOS x86-64 (Rosetta), Linux arm64 and Linux x86-64 (Docker) | high | `cargo test -p ep9_frontend --release --lib bytecodegen::corpus -- --ignored --nocapture` | — | Not the tools/test262 subcommand |
| BCG-SCOPE-ATTR | Three `#[cfg_attr(not(test), expect(dead_code, ...))]` attributes in `crates/ep9_frontend/src/scope/mod.rs` are removed. BytecodeGen now calls those functions, so `expect` fails the build | high | `cargo clippy -p ep9_frontend --all-targets -- -D warnings` | — | Outside the bytecodegen path; one-line change each |

**VM contracts.** BytecodeGen v0 assumes these. The VM agent must check them.

| Contract | Where it shows |
|---|---|
| `StaGlobal` and `SetNamedProperty` take strictness from the function flag | sloppy vs strict goldens |
| The VM checks TDZ and `const` for global lexical bindings, also in `LdaGlobalInsideTypeof` | `var-global-lexical` |
| Module cells start as the hole; imports are hole-checked by the VM | `module-*` goldens |
| Accessor runtime arguments that are `undefined` mean "absent" (`DefineAccessorInLiteral`, `DefineClassAccessor` merge with the other half) | `lit-object-methods`, `class-base` |
| New function-context slots and registers start as `undefined` | all goldens |
| Mapped `arguments` needs a parameter-to-slot map | `var-arguments` |
| `DefineClass(ctor, heritage)` sets the prototype, `constructor` and the home object of the constructor | `class-*` goldens |
| `ResolvePossiblyDirectEval` receives the unit scope id of the call site | `call-eval`, `var-direct-eval` |
| `DeclareGlobals` reads the declarations from unit scope 0 | every script golden |
| RegExp flag bit i = letter i of `dgimsuvy` | `lit-regexp` |

**Known gaps (open).** `with (o) f()` calls `f` with an `undefined` receiver. The default derived constructor and a spread call that is not last iterate an array (observable). Script completion values are not kept (f0 returns `undefined`). Module hoisted functions are set at f0 start, not at module instantiation (H02-Q3). The `super` base is read at access time.

## 9. 2026-10-21: PR #1 build decisions and doc sync

**Source.** The docs-sync agent of the M0 workflow, under the user's delegation of 2026-10-20 (§5). The agents that built PR #1 (branch `m0/weeks2-3`, commit `fdad2b4`) reported the choices in §9.1; no earlier section records them. Each row was checked in the code on 2026-10-21. The Evidence column names the place. All reported items were confirmed; none was dropped.

### 9.1 Decisions in the PR #1 code

| ID | Decision | Conf. | Evidence | Reopen rule | Note |
|---|---|---|---|---|---|
| T0-NOT-IMPL | An opcode or path that T0 v0 does not run throws a catchable `Error` with the message "not implemented in ep9 v0: <what>". Never a panic, never UB | high | `crates/ep9_vm/src/error.rs` (`not_implemented`); `src/interp/ops.rs` (module doc); test `tests/exceptions.rs::type_errors_and_not_implemented_are_catchable` | Each item goes when its feature lands | Every v0 shortcut in `ops.rs` is marked `// v0:` |
| T0-NATIVE-WORD | A native word on the VM stack (caller fp, return pc, FunctionInfo) is stored as `addr + 2^49`, a subnormal double pattern. So every slot in `[base, top)` is a valid `Value`, and the GC scans the VM stack as one precise slice | high | `crates/ep9_vm/src/frame.rs` (`native`, `native_addr`); `DOUBLE_ENCODE_OFFSET = 1 << 49` in `ep9_value` | A target with addresses at or above 2^49 (debug assert in `native`) | Decoding is one subtract. Object cells use the same encoding (`object.rs`) |
| T0-STACK-LIMIT | Both stack limits give a catchable `RangeError` "Maximum call stack size exceeded": the VM register stack cap (`push_frame` finds no room) and the native stack bound (`sp > native_limit`; `native_limit` = stack low end + clamp(stack size / 4, 128 KiB, 1 MiB)). The isolate stays usable after it | high | `crates/ep9_vm/src/call.rs`; `runtime.rs` (`native_stack_ok`); `isolate.rs` (`native_margin`); tests `vm_stack_overflow_is_a_catchable_range_error` and `native_stack_overflow_is_a_catchable_range_error`, every dispatch kind | The H23 ADR (limits) sets other values | The native check runs at JS entries, host calls and recursive runtime routines |
| T0-DEBUG-STACK | Debug builds only: the threaded handlers (sibling call, `become`) panic when the native stack pointer reaches 64 KiB below `native_limit` (`DEBUG_SLACK`). So the RangeError path keeps 64 KiB to unwind. The panic text says to use `Dispatch::Match` | high | `crates/ep9_vm/src/interp/dsl.rs` (`debug_native_stack_check`) | — | Debug builds have no tail calls, so sibling dispatch grows the native stack by one frame per instruction |
| T0-COLD | Match loop (the x86-64 release expansion, H15): each cold block is an out-of-line `#[cold]` `#[inline(never)]` function (`cold::*`). Each arm stays small, so LLVM can copy the dispatch into the arms | high | `crates/ep9_vm/src/interp/dsl.rs` (`expand_match!`); `cargo xtask disasm-check --match-loop` | The `--match-loop` check fails on T0 (H15-X64 reopen rule, §7) | **Corrected 2026-10-21 (§10 H15-X64-T0):** the match loop is the debug expansion; LLVM does not copy its dispatch on T0; the cold functions stay. The threaded expansions have one cold handler per cold block |
| T0-NEW-TARGET | v0 keeps new.target in the FEEDBACK header slot of a `[[Construct]]` frame (argc bit 30). `LdaNewTarget` reads it there. Host functions get it as an argument. The slot is free until H16 | medium | `crates/ep9_vm/src/call.rs` (`push_frame`); `src/interp/ops.rs` (`LdaNewTarget`) | H14-Q1 (open; M1: classes, `Reflect.construct`); H16 needs the slot | The H14 text was fixed the same day (clarify) |
| VM-CELL-KINDS | v0 cell kind ids: STRING 1, OBJECT 2, FUNCTION 3, ARRAY 4, ERROR 5, PROPS 6, ELEMENTS 7, CONTEXT 8, ITERATOR 9 (array and string iterator, 48 B). A FUNCTION cell has 56 B: object (32 B), FunctionInfo (native word), context, `[[HomeObject]]` | high | `crates/ep9_vm/src/object.rs` (kind table, `mod fun`, `mod iter`) | Shapes (H8, H17) and H19 strings replace the v0 object model (week 4) | Kind 0 is reserved (`ep9_heap::KindId`) |
| T262-INFRA | `ep9-test262 exec`: a panic, a timeout (soft per-run timeout, default 20 s; then a hard kill) or a worker crash is an infrastructure error, not a failure. Any infrastructure error makes the run exit 2 before it compares or writes the expectation file | high | `tools/test262/src/exec.rs` (module doc; `if infra.is_empty()` guards the compare and `--update-expectations`); usage text in `main.rs` | — | Same rule as `parse` |
| T262-NAME | The exec expectation file is `expectations/test262.<target-triple>.jit.txt` (NAMING `<suite>.<target>[.<mode>].txt`, mode `jit`). First file: `test262.aarch64-apple-darwin.jit.txt` | high | `tools/test262/src/main.rs`; [expectations/](../expectations/README.md) | NAMING reopen rule (§5.4) | |
| HEAP-LOS-VA | Each LOS cell takes a run of whole 256 KiB chunks of cage VA (at least one). It commits only up to the OS page after the cell. So the default 64 GiB cage holds at most 262,144 LOS cells (64 GiB / 256 KiB), fewer when small pages use chunks | medium | `crates/ep9_heap/src/heap.rs` (`alloc_large`); `memory.rs`; `DEFAULT_CAGE_SIZE` in `crates/ep9_platform/src/va.rs` | **Open.** Decide at the S1 design (M1) whether small LOS cells share chunks. Earlier if a test or workload hits the limit | Accepted for S0. A LOS cell is above 8 KiB (H09-Q3), so its VA cost is up to about 30 times its size |
| HEAP-SIZEFN | A `CellSize::Dynamic` size function may read only its own cell | high | `crates/ep9_heap/src/cell.rs` (`SizeFn` doc) | — | The heap calls it on cells found through a VO bit or a LOS header (`cell_size`) |
| HEAP-SCRATCH-LIFO | `ScratchRoots` scopes nest LIFO, checked at run time: a push into a scope that is not the innermost panics, and a scope dropped out of order panics (not while a panic unwinds) | high | `crates/ep9_heap/src/roots.rs`; test in `src/tests/gc.rs` (`not the innermost`) | — | 03-gc §2 `ScratchRoots` |
| OXC-VISIT-PIN | `oxc_ast_visit = "=0.155.0"` moves to the root `[workspace.dependencies]`; `ep9_frontend` uses `workspace = true`. The root now pins 7 oxc crates | high | `cargo metadata --locked` and `cargo build -p ep9_frontend --locked` pass; `Cargo.lock` unchanged (sha256 `e88aee50...f94cd8b`) | The next oxc bump PR bumps all 7 together | Closes H01-Q1 (fill). The §2 note "D1 in the repo" (6 crates) was right until this change |

### 9.2 Corrections to earlier entries

| Where | Old text | Text now | Why |
|---|---|---|---|
| §7, row REMOTE-CI (note) | Decision: "The user allowed commits and pushes to the private repo on 2026-10-20/21. The workflow task text quotes the user: "You are free to create repo, keep it private first"; "Keep working on our plan" (workflow script text; the H15 agent did not see the user messages). This settles the REMOTE-CI critic note "commit and push only on an explicit user instruction". The repo stays private; no force-push to `main`". Evidence: "user messages as quoted by the workflow task". Note: "Records the instruction that the §5.4 REMOTE-CI row asks for" | The user wrote both messages in the session (times in UTC). Evidence: the session transcript. The note adds the correction | The old text was wrong. The transcript shows the user's own messages (§9.3). The decision does not change: the repo stays private; no force-push to `main` |

The row was edited in place, as in §6. This table keeps the old text.

### 9.3 User statements behind §2, §5, §7 and §8

Some entries mark a quote as relayed by the workflow harness or as workflow script text. That was how the agent got the text. The session transcript (`53ce3b1c`, checked 2026-10-21) shows that the user typed each quote.

| Quote (verbatim) | Time (UTC) | Used in |
|---|---|---|
| "go, let's start with your ultracode" | 2026-10-20 12:20 | §2 |
| "Don't ask me questions. Deepresearch and descide yourself" | 2026-10-20 14:24 | §5, §8 |
| "You are free to create repo, keep it private first" | 2026-10-20 14:27 (sent while a turn ran) | §7 REMOTE-CI (note) |
| "Don't stop, I'm going to sleep. Keep working on our plan" | 2026-10-20 16:32 | §7 REMOTE-CI (note), §8 |
| "What are u watching? CI is already finished and failed" | 2026-10-20 17:01 | §7 |

### 9.4 Follow-ups applied (no decision changes)

| From | Done on 2026-10-21 |
|---|---|
| §7 PERF-NOISE (amended) | [10-quality §3](plan/10-quality.md#3-benchmark-sets) row S: "The result names its CPU model and holds for that model only" |
| §7 D21-UNIT | "18 sibling-call dispatches" became "18 release-expansion dispatches of the loop kernel" in 04 §5 (the row said §4), 09 §4 and §8, 11 §3.2 (D21) and §4 |
| §6 "Scratch evidence" | H11 and H36 evidence [M] link the copies in [research/artifacts/decide/](research/artifacts/decide/README.md) |
| §5.4 LICENSE | `tools/bench` has `license.workspace = true`; `spikes/sp-dispatch` states `license = "MIT OR Apache-2.0"`. [12-governance §1.1](plan/12-governance.md#11-decision) updated |
| §5.4 UPSTREAM, GITATTRIBUTES | 12-governance §5.1: the three drafts are "draft (2026-10-21)", with links. §6: `sync-lib.sh` sets `core.autocrlf=false` (PR #1) |
| ADR index | [adr/README §5](adr/README.md#5-index-of-h1-h37): H1, H2, H3, H5, H14 Draft (2026-10-21); H28 notes SP-JIT-mem; counts 17 Draft, 20 Not started |
| PR #1 code | H07 notes the macOS arm64 `MAP_JIT` code range (RWX from reservation). H14 states where v0 keeps new.target |

## 10. 2026-10-21: PR #1 review fixes, CI gates and H15 on T0

**Source.** The follow-up agent of the M0 workflow, under the user's delegation of 2026-10-20 (§5). The rows come from the PR #1 review fixes (commits `9d2a0fb` to `2111b19` on `m0/weeks2-3`, the base of `m0/followups`) and from two branches merged into `m0/followups`: `ci/m0-gates` (`b1b0925`) and `h15/t0-x64` (`7b92f7d`). Each row was checked on 2026-10-21 against the code, the commit messages, the review findings or the cited data. The Evidence column names the place.

```text
PR #1 review (Codex on 4 slices; Claude on platform) -> 10.1   16 fixes + 2 platform fixes
ci/m0-gates (b1b0925) + this follow-up               -> 10.2   CIG-1..CIG-9
h15/t0-x64 (7b92f7d)                                 -> 10.3   x86-64 release = sibling calls
in-place edits to §7 and §9.1                        -> 10.4
```

### 10.1 PR #1 review fixes

**Review.** A Codex adversarial review ran on 4 slices of PR #1: bytecode (B), vm and test262 runner (V), heap (H) and frontend (Fr). The Codex service filter blocked the platform slice twice, so a Claude agent reviewed `ep9_platform` (P). Each finding came with a reproduction or with code evidence (Fr3: the emitted control flow). All 16 Codex findings were fixed: B1-B2, V1-V5, H1-H3, Fr1-Fr6. The platform review gave P1 and P2.

| ID | Decision | Conf. | Evidence | Reopen rule | Note |
|---|---|---|---|---|---|
| B1 | A handler target must be at or after its range end. A throw then only goes forward, and every loop inside a function passes a `JumpLoop` (an interrupt poll). Chosen over an interrupt poll on the unwind path: it keeps "only `JumpLoop` goes back" (H02 F6), adds no poll or deopt point and needs no VM change | high | `crates/ep9_bytecode/src/verify.rs` (guarantee table, "target before the range end"); test `verifier::handler_back_edge_does_not_load` (before: a `Throw` loop that `terminate()` did not stop); 72,147 corpus units still verify | A construct needs a handler that targets code before its range | No ABI bump: the format does not change, and ABI 1 and 2 never shipped. [H02](adr/H02-bytecode-abi.md) change log row |
| B2 | The verifier enforces the reader's rules: canonical strings, defined scope and function flag bits. So a unit from `UnitBuilder::finish` always loads back | high | `UnitString::is_canonical`, `ScopeFlags::is_defined`, `FunctionFlags::is_defined`, used by the verifier and the reader; test `verifier::builder_output_meets_the_reader_rules` | — | H02 change log row |
| V1 | A class constructor throws a TypeError on [[Call]] from Rust (`call_internal`: ToPrimitive, `HostCx::call`, `Isolate::call`), as the `Call*` slow path does (ECMA-262 10.2.1 step 2). Construct from Rust is unchanged | high | `crates/ep9_vm/src/call.rs`; test `calls::class_constructor_needs_new_on_rust_calls` (all dispatch kinds, with and without GC stress); e2e `classes/coerce_without_new.js` | — | Before: `{ valueOf: C } + 1` ran C's body |
| V2 | test262 exec: a run whose watchdog deadline passed is a timeout infrastructure error, even if it returns (pass, fail or harness error) | high | `tools/test262/src/exec.rs` (watchdog `expired`); test hook `EP9_TEST262_FAULT=late` | — | A native loop that does not poll termination can outlast the deadline and then return |
| V3 | An async test fails on any `Test262:AsyncTestFailure:` line, before or after a completion. It passes only on exactly one `Test262:AsyncTestComplete` line, so a second `$DONE()` is a failure | medium | `exec.rs` (async check); INTERPRETING.md (test262 `d53ebc8b`) says nothing about a second call | test262 states a rule for a second `$DONE()` | Product call: a duplicate callback is an engine bug worth seeing |
| V4 | A runtime negative test passes only if the thrown value's constructor name is the expected type. The message text does not count. `ep9_vm::Exception` gets one field, `constructor` (`v.constructor.name`, data properties only, runs no JS) | high | `exec.rs` (`exception_matches`); `crates/ep9_vm/src/isolate.rs`; test `negative_types_match_the_constructor` | — | Before: `throw "TypeError: x"` passed a TypeError test |
| V5 | `CodegenError::Internal` (a builder or verifier rejection) is an infrastructure error, for the test, a negative parse test and harness files. So `--update-expectations` can never list it | high | `exec.rs`; test hook `EP9_TEST262_FAULT=internal`. Full run unchanged: 9,767 / 53,616 pass, 0 infrastructure errors | — | |
| H1 | `alloc(kind, size)` checks the size of a Fixed kind before any path runs (TLAB, overflow TLAB, LOS) and returns `AllocError::WrongSize`. A release check, not a debug assertion | high | `crates/ep9_heap/src/heap.rs`; test `fixed_size_kind_rejects_other_sizes` | — | Before: a wrong size let the sweep zap a live neighbour's header |
| H2 | A panic in a root source, trace, size or weak source callback during `collect` resets the heap state, not a poisoned heap: `UnmarkOnUnwind` clears every mark bit (small pages and LOS) and the pending work lists are dropped. The lazy sweep holds a per-page guard and stores the VO word before it zaps | high | `heap.rs` (`UnmarkOnUnwind`: nothing is freed before marking ends, so clean marks are the whole state); tests `caught_{root_source,trace,weak_source}_panic_leaves_no_marks`, `caught_size_panic_in_lazy_sweep_leaves_no_marks` | A collect step frees memory before marking ends | The heap stays usable after a caught panic; the cost is on the panic path only |
| H3 | A new small page takes a cached (committed) free chunk first, else the lowest free chunk. At most 16 freed small pages stay cached (`MAX_CACHED_FREE_PAGES`). A large object evicts cached chunks in its run, and others when the limit needs it | high | `crates/ep9_heap/src/memory.rs`; tests `cached_page_is_reused_under_the_limit`, `cached_pages_never_cause_out_of_memory` | — | Before: a tight `max_heap_bytes` gave a false out-of-memory |
| Fr1 | Pre-scan of regex classes: without the v flag a class is one level; with v every unescaped `[` opens a class and every `]` closes one. The scan keeps both counts and picks one by the flags | high | `crates/ep9_frontend/src/prescan.rs`; unit test and regex gate forms at cap, cap+1 and 1M levels; e2e through `ep9` | — | Before: `/[[a]&&[[a]&&...]]/v` at 100k levels aborted with a stack overflow instead of a RangeError |
| Fr2 | Names bound by a catch pattern are TDZ bindings, initialized at the end of the pattern (as non-simple parameters); BytecodeGen stores the hole first. A plain `catch (e)` has no TDZ | high | test `scope::tests::catch_patterns_have_a_tdz`; e2e `exceptions/catch_pattern_tdz.js` (output from node) | — | ECMA-262 CatchClauseEvaluation |
| Fr3 | IteratorClose on a throw completion: the `return` lookup and the call run under the handler that ignores errors, so the original exception stays (ECMA-262 7.4.11 step 4). The normal path keeps both errors | high | test `bytecodegen::tests::iterator_close_on_a_throw_ignores_errors_of_the_return_lookup`; e2e `iter/iterator_close_return.js`; 9 goldens, 3 more instructions per IteratorClose | — | The v0 VM has no accessors, so the getter case cannot run yet |
| Fr4 | `super[k]` keeps the key value; GetValue and PutValue convert it (`LoadKeyedFromSuper`, `StoreToSuper`). So `super[k] = v` converts after `v` runs. Compound, logical assignment and update convert once, before the read | high | test `super_keys_are_converted_by_get_value_and_put_value`; e2e `classes/super_key_order.js`; test262 `target-super-computed-reference.js` passes | — | Spec and JSC (bun 1.4.3); node 24 converts twice, ep9 does not follow node here |
| Fr5 | `(o?.m)()` and `` (o?.m)`x` `` keep `o` as the receiver: a parenthesized chain that ends in a property access is still a reference. A nullish base gives an undefined callee, and the call throws a TypeError after the arguments run | high | golden `call-optional-paren`; e2e `objects/optional_call_receiver.js`; test262 `optional-call-preserves-this.js` passes | — | |
| Fr6 | Corpus checks assert their claims and fail when a third_party checkout is missing or empty (the message names the sync script): scanner, scope pass (mismatches exactly SplitBody 54, AnnexB 488, CatchVar 4), P4, `measure_forms`, bytecodegen corpus | high | `crates/ep9_frontend/src/probe.rs` (`assert_checkout`); commit `2111b19` | A pin bump changes a count: the PR that bumps updates the numbers | Before: they printed problems and passed with 0 files |
| P1 | `EhFrameRegistration::register` Safety: drop a registration only when no thread unwinds or walks through that code, and the spans (lowest `pc_begin` to highest FDE end) of two live registrations must not overlap. Doc-only | high | `crates/ep9_platform/src/unwind/unix.rs`; probes: a drop during a walk gives SIGSEGV in libunwind (macOS arm64), abort or SIGSEGV with libgcc 12 (Linux arm64, x64); interleaved spans lose a frame with libgcc 12 and 14 | — | **Open, unverified:** the Windows `FunctionTableRegistration` contract has no drop-during-walk rule and likely has the same race |
| P2 | `native_return_addresses(0)` returns no frame (the doc says "at most `max`") | high | test `unwind::tests::native_return_addresses_honors_max` (commit `9d2a0fb`) | — | Bug fix, listed for completeness |

### 10.2 CI gates (`ci/m0-gates` and this follow-up)

| ID | Decision | Conf. | Evidence | Reopen rule | Note |
|---|---|---|---|---|---|
| CIG-1 | `test262 exec` runs in the test262 job (renamed from `test262-parse`), after `parse`, on the 4 tier-1 runners, against `expectations/test262.<host-triple>.jit.txt`. Exit 1 (a difference) and exit 2 (an infrastructure error) fail the job. A failed job uploads the `--json` output | high | `.github/workflows/ci.yml` job `test262`; proof run 39502316082 (all 21 jobs green, commit message `b1b0925`) | The job passes its 60-minute timeout | exec runs only when the test262 sync step succeeded |
| CIG-2 | The x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu and x86_64-pc-windows-msvc files come from the `--json` artifacts of CI run 39501063919 (branch `diag/ci-gates`). All 4 targets fail the same paths with the same cause and detail | high | [expectations/README](../expectations/README.md); Docker (Linux arm64, Linux x64 on Rosetta) writes the same bytes. 43,849 paths; 43,847 after Fr4 and Fr5 (`1ee8583`, `d514b6e`) | One target fails a path that another passes: explain the platform difference before any file changes | |
| CIG-3 | The `disasm` job checks the host target of each of the 4 runners, not cross targets | high | `ci.yml` job `disasm`: the machine code depends on the target, and each tier-1 target has its own runner | A tier-1 target without its own runner | Local checks still use `--all-targets` (5 targets) |
| CIG-4 | `disasm` runs `cargo xtask disasm-check` on `ep9_vm` with `--expect 282` and no `--match-loop` gate, then `crates/ep9_heap/disasm-barrier.sh` | high | `ci.yml`; H15 Rules row T0 | An opcode or a cold block is added (282 = `THREADED_HANDLERS`); a release expansion with a match loop | The CI agent's reason was "H15 is open". Since H15-X64-T0 the reason is: the match loop is the debug expansion, and its shape is a report. The `ci.yml` comment now says so |
| CIG-5 | Miri runs on every PR (job `miri`, Linux x64, `nightly-2026-10-04`) on `ep9_value` and `ep9_heap`, until the nightly trigger lands. Then it runs nightly, as 10-quality §5 says | medium | `ci.yml` job `miri` | The nightly trigger lands (REMOTE-CI scope job); Miri passes its 30-minute timeout | [10-quality §5](plan/10-quality.md#5-other-ci-checks) and [H06](adr/H06-value-encoding.md) updated |
| CIG-6 | `cargo deny --locked check licenses bans` runs in `dep-guards` from the cargo-deny 0.20.2 release binary, checked by a pinned SHA-256. No third-party action, no build | high | `ci.yml` job `dep-guards` | A cargo-deny update: bump the version and the digest together | `deny.toml` header updated |
| CIG-7 | The spikes job runs the SP-JIT-mem gate with `SP_JIT_EXTRAS=0` (host target only). The Rosetta and Windows cross-build extras are for a local Mac | high | `ci.yml` job `spikes`; `spikes/sp-jit-mem/gate.sh` | — | |
| CIG-8 | No rustdoc gate yet: 6 intra-doc links were broken | — | CI agent's report | — | **Superseded the same day by CIG-9** |
| CIG-9 | The lint job runs `cargo doc --workspace --no-deps --locked` with `RUSTDOCFLAGS=-D warnings`, and `node --test tools/bench/workloads/t0-dispatch.test.mjs` next to the `run.mjs` test. The 6 links became plain text or escaped tags; no item became `pub` for a doc link | high | `ci.yml` job `lint`; local runs pass (rustdoc exit 0; 8 node tests pass) | A rustdoc warning class that our code cannot fix | Supersedes CIG-8 |

### 10.3 H15 on T0 (x86-64 release dispatch)

Data: [measurements/2026-10-21-t0-dispatch-x64.md](measurements/2026-10-21-t0-dispatch-x64.md). Decision record: [adr/H15-interpreter-dispatch.md](adr/H15-interpreter-dispatch.md). Trigger: the §7 H15-X64 reopen rule "the x64 loop check fails on T0" fired on `ep9_vm` (157 opcodes, 282 handlers).

| ID | Decision | Conf. | Evidence | Reopen rule | Note |
|---|---|---|---|---|---|
| H15-X64-T0 | Release expansion: sibling calls on every target (`Dispatch::RELEASE = Sibling`). Debug: match everywhere | high | T0 match loop on x86-64: 2 indirect jumps (need ≥ 79), 1 of them the dispatch; 92-134 stack accesses away from calls. Match / sibling time, geomean of 4 T0 kernels: 1.21-1.47 on 22 hosted x86-64 jobs (EPYC 7763, 9V74, 9V45; Xeon 8370C, 8573C); all 110 kernel results have the interval above 1 (lowest bound 1.037). aarch64: 1.26-1.53. cachegrind: match runs 1.09-1.63x the instructions | On a named x64 model T0 match beats T0 sibling (PERF-NOISE fail, geomean of 4 T0 kernels, ≥ 31 rounds); a T0 sibling handler loses its tail jump on a tier-1 target and no DSL fix restores it; the release expansion fails against cgoto.c on a named x64 model | **Supersedes §7 H15-X64.** The spike's x86-64 result (match faster on Zen 3 and Zen 4) came from its threaded 20-opcode loop and does not hold for T0 |
| H15-HANDLER-ABI | On x86-64 the handlers are `extern "sysv64-unwind"` on every OS (`dsl::handler!`). Other targets keep the Rust ABI. The ≤ 4 argument rule stays | high | Win64 fast handlers with a frame: 34 → 12 (Linux: 11). Windows sibling time old / new: 1.13 (EPYC 7763), 1.04 (Xeon 8573C), 1.02 (EPYC 9V45); Linux control 0.996, 1.003; `ep9_vm` and `ep9_cli` tests pass on windows-2025 (run 39503403036) | A Windows tool cannot walk or unwind these frames; the gain is gone on a named model (old / new ≤ 1) | `-unwind` keeps a panic (a VM bug) unwinding. Nightly `become` builds with it |
| H15-NO-THREADED-T0-MATCH | No change that makes the T0 match loop threaded: no `unreachable_unchecked()` for a bad opcode and no global `-tail-dup-*` LLVM flags. For T0, `--match-loop` is a report | high | Probes (measurements §3): each change alone keeps 2 dispatch jumps; both together give 251 jumps but 320 stack accesses away from calls. The first is UB on a VM bug; the flags apply to the whole binary | A release expansion with a match loop is proposed again (then H15-Q2 first) | The spike's threaded-loop gate stays (SP-Dispatch only, DISPATCH-GATE (T0)) |
| XTASK-MACHO-DICE | `cargo xtask disasm-check` drops the "instructions" that `llvm-objdump -d` decodes from Mach-O data-in-code ranges (LC_DATA_IN_CODE): x86-64 jump tables in `__text` are data. In a linked image the base is the vmaddr of the segment at file offset 0 | high | x86_64-apple-darwin: `ep9loop__match__run` 8 → 2 jumps (as Linux, Windows); the SP-Dispatch loop 17 → 16; tests `macho_jump_table_is_data`, `macho_data_in_code_base` | The `llvm-objdump --data-in-code` output changes | Tool fix; listed because check results depend on it |
| H15-Q1 | Answered early: the x64 release expansion with the T0 opcode set is sibling calls, on 5 named x64 models | medium | H15 ADR, Open questions | — | **Still open:** one Intel client model and a gate-host run, by M2 exit (T0 parity) |
| DISPATCH-GATE (T0) | The §7 gate per target, with "release expansion" = sibling calls on every target. x86-64 speed gates: `sibling-vs-cgoto-x64-sysv`, `-x64-win64` (≤ 1.25x cgoto.c geomean, ≤ 1.5x per kernel). Reports, never gates: `match-vs-cgoto` on every target, `sibling-vs-match` on x86-64. The spike's x86-64 `--match-loop` shape stays a blocking structural gate (H15 Rules: the threaded-loop rule applies to SP-Dispatch, not to T0) | medium | `spikes/sp-dispatch` (`gate.sh`, `src/main.rs`, `src/vm.rs`); local gate on Apple M5 Max (load 9-13, provisional): exit 0, every structural gate passes (aarch64; static x64 SysV and Win64, negatives included), `sibling-vs-cgoto-a64` 0.985, `sibling-vs-match-a64` 1.550 | As §7 DISPATCH-GATE (amended) | **Supersedes §7 DISPATCH-GATE (amended)** (note). Speed gates still never block on hosted CI. H15-Q3 (cgoto.c compiler pin) stays open |

### 10.4 Edits made in place on 2026-10-21

| Where | Old text | Text now | Why |
|---|---|---|---|
| §7, row H15-X64, Note | "The R06 gain of 1.3-1.5x holds on aarch64 only" | Starts with "**Superseded 2026-10-21 by §10 H15-X64-T0.**"; old text kept | The T0 result replaced the decision (append-only rule: the row stays) |
| §7, row DISPATCH-GATE (amended), Note | "**change taken**: on x64 the old gate compared threaded code with threaded code. ..." | Starts with "**Superseded 2026-10-21 by §10 DISPATCH-GATE (T0).**"; old text kept | Same |
| §9.1, row T0-COLD, Note | "The threaded expansions have one cold handler per cold block" | Starts with "**Corrected 2026-10-21 (§10 H15-X64-T0):** the match loop is the debug expansion; LLVM does not copy its dispatch on T0; the cold functions stay."; old text kept | The row said "Match loop (the x86-64 release expansion, H15)" and "so LLVM can copy the dispatch into the arms". Its reopen rule fired |

### 10.5 Follow-ups applied (no decision changes)

| From | Done on 2026-10-21 |
|---|---|
| H15-X64-T0, DISPATCH-GATE (T0) | Plan rows [02 §6 H15](plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [04 §2](plan/04-execution.md#2-t0-techniques-that-reach-parity), [09 §4 SP-Dispatch](plan/09-milestones.md#4-spikes) and the [ADR index](adr/README.md#5-index-of-h1-h37) say sibling calls on every target. H15 ADR: x64 gate names, H15-Q3, follow-ups, validation, change log. `spikes/sp-dispatch`: `Variant::RELEASE`, bench gates and reports, `gate.sh`, README. `tools/xtask` doc. `ci.yml` disasm comment |
| B1, B2 | [H02](adr/H02-bytecode-abi.md) change log |
| CIG-2 | [expectations/README](../expectations/README.md): 43,847 failing files per target |
| CIG-5, CIG-6 | 10-quality §5 row Sanitizers, H06 Validation, `deny.toml` header |
| Session paths | `docs/research/` 01-15: absolute session paths (they held the OS user name) became `scratchpad/...`, `~/...` or repo-relative paths; paths of the 16 files that were copied into `research/artifacts/` became links (18 links). Rule in [research/README](research/README.md). Older commits keep the old text |

## 11. 2026-10-21: M0 week 4

**Source.** The M0 week-4 build, run by the orchestrator under the user's delegation of 2026-10-20 (§5). A design workflow (3 designers, a planner and a critic) wrote the build plan; the critic's corrected plan is the one used. The plan runs in phases P0-P4 on branch `m0/wk4`; each phase merges its agent branches, passes CI, gets a Codex adversarial review and appends its rows here.

```text
week-4 plan (design + critic)        -> 11.1
P0 scaffold (VM split, crates, ADRs) -> 11.2
process (CI blocked, local CI)        -> 11.3
```

### 11.1 Week-4 plan decisions

| ID | Decision | Reason | Evidence |
|---|---|---|---|
| W4-BASE | Base is m0/followups at 7eca206, which includes fx/exit70 (eb20c38, 8fdf4da). Every worktree links third_party/{test262,bench-v8v7,bench-octane,bench-jetstream,shells} from the main repo after a pin check | fx/exit70 fixes the random cachegrind exit 70; test262, the corpus digest and v8v7 need the checkouts; VM file:line refs are unchanged | git diff --stat 942d782 7eca206 touches only crates/ep9_platform and tools/bench; crates/ep9_platform/src/va.rs:43; main repo third_party/test262 at d53ebc8b, bench-v8v7 at c6453cd |
| W4-ABI-NONE | No bytecode ABI bump in week 4. FeedbackSlotKind keeps bytes 0-20 and its names as H16 v0; RuntimeId stays a placeholder | The fingerprint hashes the opcode text, the tiny unit and the runtime table, not doc text; no week-4 item needs a new opcode, kind or id | crates/ep9_bytecode/tests/abi.rs:16-24; abi-history.txt `2 6d92eb9a`; decisions-log BCG-CORPUS decb0871…8fce76 |
| W4-RUNTIMEID | RuntimeId has 19 ids (0-18). Accessors use the existing ids 12 and 16 | Corrects the stale brief (6 ids) | crates/ep9_bytecode/src/feedback.rs:129-182; crates/ep9_vm/src/interp/slow.rs:705 |
| W4-ABI-STEWARD | P2-FACTS owns crates/ep9_bytecode/src/feedback.rs and alone may bump the ABI, at most once, in P4 (default none) | One owner for one bump | task brief; H02 change-log rules |
| W4-PHASES | 5 phases with 3/7/6/6/3 agents and disjoint paths. P0 creates every shared touch point: module lines, FunctionInfo fields, key and string helpers, the fixed-field object API, shape/atom/bound hooks, scratch_top, and the IsCallable switch | Parallel worktrees with no cross-owner edits | crates/ep9_vm/src/object.rs (904 lines); crates/ep9_vm/src/interp/slow.rs (1,239 lines); crates/ep9_vm/src/unit.rs:286 |
| W4-DEPS-FROZEN | Dependency, dev-dependency and feature tables plus Cargo.lock change only in P0. Pre-added: ep9_vm->ep9_math, ep9_cli->ep9_builtins, test262->ep9_builtins, xtask->syn 2.0.119, ep9_unit->ep9_facts, ep9_builtins->{ep9_vm, ep9_math} with dev-deps ep9_frontend, ep9_heap, ep9_bytecode | Cargo.lock is shared by every crate change | Cargo.toml:3 (members crates/*, tools/*); crates/ep9_vm/Cargo.toml has no ep9_frontend |
| W4-EXPECT-ORCH | Only the orchestrator writes expectations/test262.*.jit.txt, from the 4-target CI JSON. P0-VM creates expectations/test262-gc-skip.txt | Parallel removals conflict; only CI covers the 4 targets | tools/test262/src/exec.rs:1158-1164 |
| W4-T262-GC-SKIP | test262 exec gets --skip-list, accepted only with --gc-stress, listing the 4 known gc-stress timeouts. Acceptance commands use set -o pipefail with tee | Infra errors (timeouts) give exit 2 and skip the expectation compare, so T262-GC could never pass; tee hides the exit code | tools/test262/src/exec.rs:1118-1180; tools/test262/README.md (155.7 s, 4 timeouts) |
| W4-KINDS-V1 | Kind table v1: strings 1-7, SYMBOL 8, JS objects 16-63, internal 64-127, SHAPE 128. is_function = FUNCTION; is_callable adds BOUND_FUNCTION. P0 switches every IsCallable site | One-compare type tests (V8 instance-type ranges); no kind id is persisted | crates/ep9_vm/src/object.rs:34-55; IsCallable sites at crates/ep9_vm/src/conv.rs:77,175,374, crates/ep9_vm/src/property.rs:378, crates/ep9_vm/src/interp/ops.rs:768-784, crates/ep9_vm/src/interp/slow.rs:511 |
| W4-WORD0-FLAGS | word0 flags: bits 0-7 kind-specific, bit 8 is-prototype, bit 9 reserved for hash-present (H12-Q1), bits 10-15 reserved | Size from the header only; room for an M1 identity hash | docs/adr/H08-object-header-and-shapeid.md; crates/ep9_heap/src/cell.rs:10-15 |
| W4-SHAPEID | ShapeId = (addr - cage_base) as u32. The region sits at the cage start and offset 0 never holds a cell. Outside Miri a shape space needs cage align >= 4 GiB, so the id equals the low 32 bits (JSC StructureID). ep9_heap defaults to 0 (off); ep9_vm asks for 256 MiB through shape::heap_config | Correct for every config, including the 8 MiB Miri reservation aligned only to 256 KiB; existing heap tests stay unchanged | crates/ep9_platform/src/va.rs:43; crates/ep9_heap/src/memory.rs:117 and the Miri reservation; WebKit StructureID.h |
| W4-SHAPE-ALLOC | Shapes use a separate alloc_shape entry; the normal alloc fast path does not change; shape chunks count as small pages for find_cell | No cost on ordinary allocation; CSS interior lookup still works | crates/ep9_heap/src/heap.rs alloc; crates/ep9_heap/src/trace.rs find_cell |
| W4-SHAPES | Transition tree with shared DescriptorArrays (V8); linear scan up to 8 keys, lazy hash index above; append and remove-last only. Per-proto root and dictionary shapes live in PROTO_INFO, null-proto ones in a ShapeState root range; no address-keyed table | Plan 06 §2.2; H12 forbids address hashing | docs/plan/06-object-model-and-spec.md §2.2; docs/adr/H12-no-address-identity.md |
| W4-LAYOUT | H17 v0 layout: +8 slots or dictionary, +16 elements, +24 kind fields, then in-object slots. Wrapper and bound kinds go through a P0 fixed-field API | Proto in the shape; no hard-coded offsets outside object.rs | crates/ep9_vm/src/object.rs:548-702 |
| W4-COMPAT-API | OBJ keeps every existing pub(crate) object, props and property signature, with the same meaning, on shapes | No caller edits in other agents' files | crates/ep9_vm/src/property.rs:228 is the only outside use of flags_of |
| W4-FN-PROPS | Function length and name are own data properties, installed by object::new_function from FunctionInfo; prototype stays lazy | gOPD and delete become correct; realm.rs needs no edit | crates/ep9_vm/src/property.rs:36-73; third_party/test262/harness/propertyHelper.js:27-35 |
| W4-IC | IC entry [key weak | value strong], 16 B; uninit -> mono -> poly(<=4) -> sticky mega; sites 0x21, 0x22, 0x24, 0x25 only (no 0x8f, no keyed-by-name); handler holders and target shapes are weak; proto_epoch is read through a pointer to ShapeState's Cell | One format for all tiers; no prototype leaks while units are never freed; no stale epoch copy; no overlap with NAT's GetIterator / docs/plan/02-architecture.md H16 row; crates/ep9_vm/src/unit.rs (LoadedUnit has no Drop) |
| W4-FEEDBACK | Per-function FEEDBACK cell, made on the 8th call or at the first budget poll; threshold 0 = eager, u32::MAX = never; fp-3 keeps new.target; the FunctionInfo feedback and invocations fields are added in P0 | V8 default of 8; FEED does not own unit.rs, where the FunctionInfo literal is built | crates/ep9_vm/src/frame.rs:56-57; crates/ep9_vm/src/unit.rs:286; crates/ep9_vm/src/interp/ops.rs:282-320 |
| W4-ACCESSORS | JSC style: an accessor bit in the descriptor, and the slot holds an immutable ACCESSOR_PAIR; owned by P3-ACC; super receivers are done by OBJ in P2 | Literals with fresh getter closures still share shapes; avoids an edit to IC-owned slow/property.rs | crates/ep9_vm/src/interp/slow.rs:705 |
| W4-STRINGS | String header: u32 length at +8, u32 hash_field at +12, data at +16. Atoms carry the ATOM flag in a weak per-isolate table (Rc<RefCell>, no borrow across an allocation, atoms::verify in debug); unit strings are interned at load | A key compare is one word; the weak source is a 'static closure | crates/ep9_heap/src/heap.rs:696; crates/ep9_vm/src/unit.rs:225-230 |
| W4-KEY-EQ | Stored keys are always atoms (intern_key on define). A lookup classifies its probe once: atoms and symbols compare by bits; a non-atom probe takes one content scan and bumps a debug counter that must stay 0 | No per-entry header loads; still correct if a producer forgets to intern | crates/ep9_vm/src/object.rs:541 |
| W4-NO-NUMBER-KEYS | No number keys in week 4. An index string stays a string atom, and hash_field caches its index (V8 Name hash-field style) | property.rs asserts is_string and array_index_of takes strings; avoids edits in OBJ- and ACC-owned files | crates/ep9_vm/src/property.rs:22,94,163,270 |
| W4-HASH | Code-unit hash with a per-process seed (EP9_HASH_SEED override). It never goes into bytecode, units, snapshots or property order | Hash flooding (Node CVE-2017-11499); H12 | docs/adr/H12-no-address-identity.md |
| W4-STRVIEW | The 28 str_view sites (25 outside object.rs) stay; new code uses flatten/view; sites move when their owner edits the file; 0 by the H19 freeze (M1a) | No edits in other agents' files this week | grep str_view crates/ep9_vm/src: 28 total, 3 in object.rs |
| W4-HOSTFN | HostFn = for<'s> fn(&mut Cx<'s>, Local<'s>, Args<'s>, Local<'s>) -> JsResult<Local<'s>>; Args is one word; JsResult<Local> is 8 B through the NonZeroU64 niche | 4 integer args fit Win64 | crates/ep9_vm/src/host.rs:23; crates/ep9_value/src/value.rs:52 |
| W4-LOCAL | Local is repr(transparent) over Value, Copy, !Send, !Sync; no handle scopes. The debug stack check caches the thread's stack bounds in a thread_local | CSS keeps it alive (H10); avoids a /proc/self/maps parse on every access | crates/ep9_platform/src/stack.rs:28; docs/adr/H10-rooting.md |
| W4-ARGS-COPY | call_args copies argc words onto the VM stack top (no heap Vec). ArgsBuilder moves a new VmStack scratch_top, and Shared::trace scans up to max(frame top, scratch_top). Copy-free forwarding waits for a tail-call path (H20-Q3, M1) | Params sit below the header, so a live host frame's args cannot be reused; slots above the frame top were not roots | crates/ep9_vm/src/frame.rs PARAM0 = -10; crates/ep9_vm/src/runtime.rs Shared::trace; V8 Builtins::Generate_FunctionPrototypeCall |
| W4-ABI-SURFACE | P2-BTAB owns abi/** (except local.rs) and lands every entry point P3 needs. ObjectOps adds to/from_property_descriptor, to_length, to_integer_or_infinity, create_list_from_array_like, ordinary_create_from_constructor and get_prototype_from_constructor. PropDef keys are Atom, Str or WellKnownSymbol; values add WellKnownSymbol and Alias | P3 agents own no abi file; Alias makes Array.prototype[@@iterator] === values | third_party/test262/harness/propertyHelper.js:27-35; bench-v8v7 deltablue |
| W4-BUILTIN-IDS | BuiltinId u16 = module block (7 bits) + index (9 bits), append-only per block, with a golden per module | Parallel agents never shift each other's ids (H13) | docs/plan/02-architecture.md H13 row |
| W4-BUILTINS-CRATE | One ep9_builtins crate (forbid unsafe) whose engine deps are ep9_vm and ep9_math, until M1a | Math templates need round_js, fround and pow_js; ep9_math has no deps | docs/plan/08-repo.md crate table |
| W4-SYMBOLS-WRAPPERS | Symbols and primitive wrappers land in P2. Symbol identity hashes come from a per-isolate PRNG (H12-Q3 default) | Top test262 causes; the Number, String and Symbol templates need them | crates/ep9_vm/src/interp/slow.rs:214; docs/adr/H12-no-address-identity.md Q3 |
| W4-BENCH-HARNESS | Week-4 perf uses the generated v8-v7 stub harness; base.js moves to month 2 | Only richards and deltablue gate M0 | third_party/bench-v8v7/bench/bench-v8/deltablue.js |
| W4-MATH-SCOPE | Week 4 ships the ep9_math skeleton (fma_exact, fmod, round_js, fround), pow_js v0 (H36 special cases + exact path, used by ** and Math.pow), the root clippy.toml and fp-check. CR exp/log/pow and tools/math-oracle move to month 2 | The H36 Rules row puts CR functions in M1; a tools/ dir without a Cargo.toml breaks the tools/* glob | crates/ep9_vm/src/numconv.rs:184-305; Cargo.toml:3,9 |
| W4-FPCHECK | fp-check scans the post-LTO ep9 object per target on a Linux host. Fixture tests are #[ignore] and run with --ignored in the fp-check job. Fixtures have their own [workspace] and no crates.io deps | Fixtures would fail on mac/win build-test runners without llvm-tools; nested crates break the workspace | .github/workflows/ci.yml:90 (cargo test --workspace on 4 runners); no cc crate in Cargo.lock, so object-only cross builds work |
| W4-FACTS | One ep9_facts crate owned by P2-FACTS. No type holds a ShapeId or an address. FACT_SCHEMA_VERSION = 1 is exported from P0 so the ep9_unit header can read it | ShapeIds are run-local (H8); H35 S3/S4 include the fact-schema version | docs/adr/H35-aot-unit-format.md S3-S4 |
| W4-VERSIONS | VM-side contract versions live in crates/ep9_vm/src/versions.rs: object layout 1, IC 1, frame 1, builtin ABI 1, deopt 0 | One place for the H35 header; deopt 0 blocks native text | docs/adr/H35-aot-unit-format.md:51-54 |
| W4-UNIT-HEADER | 256 B little-endian unit header; page size is its own field; check() is a pure function | Shows which contract changed | docs/adr/H35-aot-unit-format.md:114-120 |
| W4-AOT-WRITER-A2 | No unit writer in week 4. The aot feature holds an empty module, and tests build units with a test-only encoder | The brief asks for an empty aot feature; the writer is A2 | task brief AOT line; docs/plan/09-milestones.md §8 |
| W4-FEATURES | jit chain ep9_cli -> ep9_vm -> ep9_platform; aot only on ep9_unit; the feature matrix runs per package, as a build-test step on 4 runners | Workspace builds unify features and would hide a jitless leak | .github/workflows/ci.yml:89-90; .github/scripts/dep-guards.sh check 5 |
| W4-PANIC-ABORT | Do not set panic = abort in week 4. H23 records the rule: abort in shipped builds from the first JIT or AOT frame (Spike B), after test262 and the frontend worker stop relying on catch_unwind | test262 parse/exec and the frontend worker catch panics in-process; an abort ends a whole run (0xC0000409 on Windows); no JIT frame exists yet | tools/test262/src/exec.rs:680; tools/test262/src/parse.rs:167; crates/ep9_frontend/src/worker.rs:96; Cargo.toml:38-42 |
| W4-H07-REWRITE | Rewrite H07 (Draft) now: one cage per isolate, as built; a process-wide code range to be built before Spike B | H07 otherwise contradicts the code and H22 for 2 months; a Draft may be rewritten | docs/adr/H07-address-space-and-code-range.md:46; crates/ep9_heap/src/memory.rs:117; docs/adr/README.md §3.3 |
| W4-H22 | H22 draft: one cage per isolate, a process code range (proposal), a realm cell with a stable intrinsics table; Q1-Q8 | Matches the code and the rewritten H07 | crates/ep9_vm/src/isolate.rs:128-133; crates/ep9_platform/src/va.rs:37-40 |
| W4-H18 | Add an H18 draft with v0 notes (P0-DOCS) | The wk4 check needs H14-H20 drafted | docs/plan/09-milestones.md §8 |
| W4-ADR-FILLS | AOTU fills H09-Q4 (no baked page mask in v0) and H11-Q3 (BARRIER_VERSION 2 in the H35 header). LINT fills H12-Q1 (defer to M1, reserve bit 9), H12-Q2 (review rule) and H12-Q3 (per-isolate PRNG) | These questions are due in week 4 and had no owner | docs/adr/H09-pages-and-side-metadata.md:75; docs/adr/H11-barrier-and-atomics.md:117; docs/adr/H12-no-address-identity.md |
| W4-LOG-APPEND | decisions-log rows are append-only; a changed decision is a new row naming the old one (VM-CELL-KINDS-v1, BCG-CORPUS updates). Change kinds: H02-Q5 fill, H14 fill, 02 H20 rewrite (before the freeze) | The log rule and the ADR change-kind rules | docs/decisions-log.md header; docs/adr/README.md §3.2-3.3 |
| W4-GC-RULES | Ground rules: no RefCell borrow across an allocation; Rc tables for root and weak sources; values live only in registers, below scratch_top, in ScratchRoots or in Rust locals; no Drop in heap types; atoms::verify instead of a heap verify hook | Root and weak sources are 'static closures that borrow these tables; ep9_heap has no verify-hook API | crates/ep9_heap/src/heap.rs:690,696; crates/ep9_heap/src/verify.rs:58 |
| W4-STUB-HYGIENE | Each P0 stub module carries #![allow(dead_code)] // wk4-stub until filled; P4-GATES checks none remain | CLIPPY runs with -D warnings | .github/workflows/ci.yml:61 |
| W4-BENCH-BASE | Every BENCH compare uses the agent's own base commit, built the same way, on the same host, in the same session. The HEAP mark-time check becomes SP-GC gate + BENCH(ci-v0) <= +1% | The run-1 report is cross-engine wall time; GcStats has no mark time | tools/bench/src/counter.rs; crates/ep9_heap GcStats |
| W4-PERF-PROTOCOL | Class R counts instructions retired (macOS time -l counts the whole process; cachegrind counts user only; compare within one counter). Class M uses paired rounds. Gates are QuickJS and node --jitless. Run 1 on P0, run 2 on P3 | PERF-NOISE rule | docs/plan/10-quality.md §3; tools/bench/src/counter.rs |
| W4-FEEDBACK-KNOB | --feedback-threshold N in the ep9 CLI and test262 exec; the eager run reuses the default expectation file | One expectation list | tools/test262/src/main.rs flag table |
| W4-CI-COST | New checks are steps in existing jobs: feature-matrix in build-test, eager in test262, gc-stress+eager in the linux-x64 test262 job. fp-check is the only new job, on Linux | REMOTE-CI budget USD 50/month; macOS minutes cost the most | .github/workflows/ci.yml job list; docs/plan/12-governance.md §3 |
| W4-SPIKE-DISPATCH | SP-Dispatch: GO on structure; speed GO with conditions (quiet-window x64 sysv/win64 runs and a re-run of the void a64 match ratio before M0 exit) | The x64 speed gates have not run as gates | spikes/sp-dispatch/README.md; decisions-log DISPATCH-GATE (T0) |
| W4-REVIEW | Each phase merge runs /codex:adversarial-review --model gpt-6.1-sol in the background; act only on findings that serve the goal or fix a defect the phase introduced | The user's standing rule against scope drift | user global instructions |

### 11.2 P0 scaffold

Branches `m0w4/p0-infra`, `m0w4/p0-vm`, `m0w4/p0-docs`, merged into `m0/wk4` in that order. All acceptance checks of the plan passed (41 checks); handlers stay 282; the test262 result is unchanged (43,847 failing files).

| ID | Decision | Reason | Evidence |
|---|---|---|---|
| W4-INFRA-SYN2 | xtask uses syn 2.0.119 (features full and visit), as the plan says, not syn 3.0.9, which is also in the lock | Neither version adds a package to the lock. syn 2 parses every Rust file in the tree, including `become` and let-chains | Scratch probe: syn 2.0.119 parsed 140 of 140 .rs files under crates/ and tools/, 0 failed (scratchpad/p0infra-probe/synprobe) |
| W4-INFRA-SPANLOC | xtask also depends on proc-macro2 1.0.107 (span-locations) and quote 1.0.47 | Outside a proc macro, gc-lint can print file:line only with span-locations. quote's ToTokens prints a matched type. The dependency tables are frozen after P0. Both crates are already in the lock | proc-macro2-1.0.107/src/lib.rs:493-495 (`#[cfg(span_locations)] pub fn start`); build.rs:42-46 |
| W4-GUARD-FEATURES | Guards 9 and 10 read each package's enabled features (cargo tree -f '{p}/{f}', with dev edges) and include a positive control | The forward tree from `cargo tree -e features` does not show features that the root package's own feature table turns on | `cargo tree -p ep9_cli -e features / grep -c 'ep9_platform feature "jit"'` = 0 for the default build; the `-f '{p}/{f}'` line shows `ep9_platform v0.0.0 /jit,unwind` |
| W4-GUARD-WS | dep-guards check 0: every directory a members glob matches, hidden directories included, has a Cargo.toml or is under an exclude path. It runs first and stops on failure | Without a Cargo.toml in such a directory, every cargo command fails | Probe: tools/py with no Cargo.toml gives cargo metadata exit 101; tools/.hidden and an empty dir also give 101; the same dir in exclude gives 0 |
| W4-GUARD-DEPKINDS | Checks 6-8 count normal and build dependencies; dev-dependencies are allowed. Check 8 is strict: every non-dev dependency of ep9_builtins, crates.io crates included, must be in {ep9_vm, ep9_math} | This matches P2-BTAB's acceptance, which says `cargo tree -p ep9_builtins -e normal --depth 1` shows only ep9_vm and ep9_math | Negative tests: an extra build dependency fails check 6; an ep9_facts dev-dependency on ep9_vm passes |
| W4-XTASK-SHARED | disasm.rs also makes these pub(crate): Block and Insn with their fields, host_triple, repo_root and ALL_TARGETS | parse_disasm returns Vec<Block>, so fp-check needs the fields. fp-check scans the same 5 targets. P1-MATH cannot edit disasm.rs | tools/xtask/src/disasm.rs |
| W4-H07-REWRITE (applied) | H07 Decision and Rules: one cage per isolate, as built; one process-wide code range, built before Spike B; the old text stays only in the change log | Before this, H07 contradicted the code and H22 | crates/ep9_heap/src/memory.rs:117; crates/ep9_heap/src/trace.rs:310-311; docs/adr/H07-address-space-and-code-range.md change log, line 104 (the plan's evidence 'H07...md:46' now points at other text: cite line 104) |
| W4-H22 (applied) | H22 draft: one cage per isolate (Q1 filled), process code range (Q2 via W4-H07-REWRITE), realm cell with a stable-index intrinsics table (Q3, proposed), Q4-Q9 with proposed defaults | Matches the code and the rewritten H07 | crates/ep9_vm/src/isolate.rs:128-133; crates/ep9_heap/src/heap.rs:147-163; crates/ep9_platform/src/va.rs:37-40 |
| W4-H23 (new) | H23 fills: out of memory = near-limit callback, then fatal abort (months 2-3); size limits throw RangeError and the isolate stays usable; embedder interrupt callbacks never run JS; a native stack overflow past the margin is fatal, with the mechanism in H28 (H28-Q3 filled: policy H23, mechanism H28) | Plan task P0-DOCS 2. Today ep9 installs no signal handler and Rust std aborts on a guard-page hit (exit 134, measured); OOM panics; there is no heap limit | docs/adr/H23-limits-and-failure-policy.md; crates/ep9_vm/src/runtime.rs:289-296; crates/ep9_heap/src/heap.rs:72 |
| W4-PANIC-ABORT (applied in H23-Q1) | Release builds keep unwinding until the first JIT or AOT frame (Spike B); after that, panic = "abort". Before the switch, test262 exec and parse, the frontend worker and the ep9 exit-70 path must stop relying on catch_unwind | No JIT frame exists yet; catch_unwind sites exist in-process | tools/test262/src/exec.rs:680; tools/test262/src/parse.rs:167; crates/ep9_frontend/src/worker.rs:96; crates/ep9_cli/src/run.rs:179-183; root Cargo.toml lines 38-42 |
| W4-FEATURES (applied in H27) | jit chain ep9_cli -> ep9_vm -> ep9_platform (jit implies unwind); aot only on ep9_unit (empty); jitless = -p ep9_cli --no-default-features; matrix per package, never --workspace; guards 5, 9, 10 | Cargo unifies features across one build, which hides a jitless leak | crates/ep9_platform/Cargo.toml:27-34; .github/scripts/dep-guards.sh:57-62 |
| W4-H18 (applied) | H18 draft: the plan's elements kinds plus v0 notes (one dense form, 2^26 cap; an index at or above 2^26 becomes a string-keyed property and length does not change; a high dense index allocates the whole range) | The week-4 check needs H14-H20 drafted | crates/ep9_vm/src/object.rs:85-87; crates/ep9_vm/src/property.rs:198-200; measured 540,606,464 B peak RSS |
| W4-SPIKE-REVIEW (new) | M0 spike review: SP-GC, SP-VA, SP-Value and SP-JIT-mem GO; SP-Dispatch GO with conditions; SP-tsrs-win recorded as PASS; Spike B and SP-Unit not reviewed; no ADR reopens | Every gate step on the 4 tier-1 runners passed; no speed gate failed | CI run 39518270586 (tree = dabc38a); docs/measurements/2026-10-21-m0-spike-review.md |
| W4-SPIKE-DISPATCH (evidence added) | GO on structure; speed GO with conditions: a quiet gate-host re-run of a64 sibling-vs-match (still void) and non-void x64 sysv and win64 results per named CPU model, before M0 exit | Hosted run 39518270586: EPYC 7763 sysv 1.165 void, win64 1.151 void; M2 Pro (Virtual) cgoto 0.481 void, match 2.116 void; Neoverse-N2 match 1.734 pass, cgoto 1.001 void | artifacts sp-dispatch-gate-* of run 39518270586; spikes/sp-dispatch/README.md |
| W4-SP-TSRS-WIN (new) | SP-tsrs-win: PASS (0 local patches, 12 of 12 CI runs, image windows-2025-vs2026 20261018.275.1, Rust 1.99.1). By D2, `ep9 check` may support Windows; `ep9c build --hints` stays unix-only until the A1h re-check | The `ep9 check` part of the gate passed | docs/measurements/2026-10-20-sp-tsrs-win.md; artifact sp-tsrs-win-log of run 39518270586 (SHA-256 68701f26...86051863) |
| W4-P0-KINDS-v1 | kind ids: strings 1-7 (SEQ_LATIN1 1, SEQ_UTF16 2, 3-7 reserved), SYMBOL 8, JS objects 16-63 (OBJECT 16 .. BOUND_FUNCTION 22), internal 64-127 (PROPS 64 .. PROTO_INFO 75), SHAPE 128; is_object = 16..=63, is_string = 1..=7 | range checks are one compare pair and leave room for M1 string kinds | crates/ep9_vm/src/kinds.rs, kinds::tests::table_v1_ranges |
| W4-P0-SCRATCH | VmStack.scratch_top; GC scans [base, max(frame top, scratch_top)); cleared on host/entry frame push and on host-fn return | Rust can keep Values above the top frame (builtin ABI arg building, H20); clearing on return keeps the bytecode fast path free | frame.rs, runtime.rs Shared::trace, call.rs; tests::gc::values_below_the_scratch_top_survive_gc_stress (fails at gc.rs:174 with the scan off) |
| W4-P0-VERSIONS | pub mod versions: OBJECT_LAYOUT_VERSION 1, IC_LAYOUT_VERSION 1, FRAME_VERSION 1, BUILTIN_ABI_VERSION 1, DEOPT_META_VERSION 0 (u32) | the plan's values; pub so AOT and tools can read them | crates/ep9_vm/src/versions.rs |
| W4-P0-FIXED | new_object_with_fields(rt, kind, proto, n), fixed_get(o, i), fixed_set(rt, o, i, v); PRIMITIVE_WRAPPER Fixed 40 B, BOUND_FUNCTION Fixed 56 B with trace from +8 | the barrier needs rt; uses today's object layout | object.rs register_kinds, UNIT(ep9_vm) |
| W4-P0-ISCALLABLE | is_callable = FUNCTION or BOUND_FUNCTION; FunctionInfo readers and HOME-slot users keep is_function; bound [[Call]]/[[Construct]] return not_implemented until P3 | spec IsCallable without touching the hot call_fast path | 12 is_function sites listed in acceptance; DISASM stats unchanged |
| W4-P0-KINDREG | object::register_kinds registers its own kinds and calls string/props/symbol/shape/feedback/accessor/ic::register_kinds | one registration point (I23); later owners fill their own module | object.rs |
| W4-P0-KEYS-TEXT | key_text(rt, key) -> String (lossy text, or describe for non-strings); key_hash = FNV-1a over UTF-16 LE bytes (stub); Probe {Bits, Content}; PropertyKey = u64 | error messages no longer need str_view; P1 replaces the stubs | key.rs |
| W4-P0-SKIPLIST | --skip-list only with --gc-stress, rejected with --update-expectations; listed tests are not run, not counted, and removed from the expected set before diff; warns on entries that name no test; prints 'skipped: N'; JSON gets 'skipped' | keeps the 4 GC-stress timeouts from making exit 2 without hiding real failures (all 4 fail anyway in every jit expectation file) | T262-GC exit 0 with the list, exit 2 without |
| W4-P0-FEEDBACK-KNOB | IsolateConfig::DEFAULT_FEEDBACK_THRESHOLD = 8, threaded to Cx.feedback_threshold through the CLI and the test262 workers; unused until P2-FEED | one knob for tests and T262-EAGER | T262 --feedback-threshold 0 exit 0, same counts |
| W4-P0-STUBSIG | stub signatures: string::{flatten(cx,s)->u64, view(cx,s)->JsStr, char_at(rt,s,i)->u64}, wrapper::{new_wrapper(rt,proto,value)->u64, wrapper_value(v)->Option<u64>, string_own_property(rt,o,key)->Option<(u64,i32)>}, bound::{is_constructor(v)->bool, new_bound_function(cx,target,this,&[u64])->Res<u64>}, atoms::{intern(rt,u64)->u64, intern_str(rt,&str)->u64, init_static(rt)}, symbol::well_known(rt,WellKnownSymbol)->u64, feedback::{UnitFeedback::new(rt,n), unit_slot(unit,id)} | fixed now so P1-P3 owners do not change shared call sites | the listed modules |

### 11.3 Process

| ID | Decision | Reason | Evidence |
|---|---|---|---|
| W4-CI-BLOCKED | While GitHub Actions starts no job, every phase merge runs a local stand-in for ci.yml (scratchpad `ci/local-ci.sh`): fmt, clippy (host, x86_64-pc-windows-msvc, x86_64-apple-darwin), rustdoc, node tests, actionlint, tests on macOS arm64, x86_64-darwin (Rosetta), Linux arm64 and amd64 (Docker), test262 parse + exec, disasm, Miri, dep-guards, cargo-deny, the 3 spike gates, perf. PRs to main wait for real CI | Run 39519345205 on main (dabc38a): all 21 jobs "not started because recent account payments have failed or your spending limit needs to be increased". Only the owner can fix billing; the repo stays private | Local baseline on dabc38a: 22/22 steps pass |
| W4-ROSETTA-SKIPS | In the Linux amd64 container (Rosetta) only, skip `expected_outputs_match_node` and `sp_va_skips_refused_slot` | Rosetta loses the exec error of glibc posix_spawn (a missing node gives Ok + exit 127) and refuses PR_SET_SECCOMP (EINVAL). Both tests pass on real x86-64 Linux | GitHub CI build + test (linux-x64) passed both on PR #2 (run 39518270586) |
