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

## 7. 2026-10-21: H15 dispatch on x86-64 (SP-Dispatch reopen)

**Source.** The H15 agent of the M0 workflow, under the user's delegation of 2026-10-20 (§5). The round started from the user's message, relayed verbatim by the workflow harness: "What are u watching? CI is already finished and failed". Inputs: CI run [39458801246](https://github.com/Brooooooklyn/ep9/actions/runs/39458801246) (commit `ea31dcb`), the x86-64 diagnosis (branch `diag/dispatch-x64`, 3 runs) and the Windows cgoto.c fix. Data: [measurements/2026-10-21-sp-dispatch-x64.md](measurements/2026-10-21-sp-dispatch-x64.md). Decision record: [adr/H15-interpreter-dispatch.md](adr/H15-interpreter-dispatch.md).

**Trigger.** The SP-Dispatch gate failed on linux-x64: "sibling ≥ 1.2x faster than match (geomean): 0.812", and match beat sibling on every kernel. That is the DISPATCH-GATE reopen rule (§5.4). The plan says a failed spike gate reopens its ADR, so H15 reopened before its first draft.

| ID | Decision | Conf. | Evidence | Reopen rule | Note |
|---|---|---|---|---|---|
| H15-X64 | Release expansion per target: sibling calls on aarch64; the match loop on x86-64 (SysV and Win64). No DSL codegen fix. Debug: match everywhere | medium | x64 match loop has 16 dispatch jumps (LLVM tail duplication), aarch64 has 1; match / sibling time 0.77-0.81 (EPYC 7763), 0.88-0.92 (9V74), 1.03 (9V45), 1.13 (Xeon 8573C and 8370C); Windows 0.83 (7763), 0.97 (9V74); 7 probes, none fixes Zen 3 | The x64 loop check fails on T0, or `match-vs-cgoto` fails on a named x64 model; H15-Q1 at M2 exit. Sibling beating match on a model is H15-Q1 input, not a trigger (the vendor split is known) | The R06 gain of 1.3-1.5x holds on aarch64 only |
| DISPATCH-GATE (amended) | Gate per target. Release expansion ≤ 1.25x cgoto.c (geomean of 3 kernels) and ≤ 1.5x per kernel. aarch64 also: match ≥ 1.2x sibling time (geomean), ≥ 1.0x per kernel. x86-64: sibling vs match is a report, never a gate. x86-64 loop shape (`disasm-check --match-loop`) is a structural gate. M0 exit item 5 on x64: no recorded CPU model fails; marginal and void runs are repeated | medium | 09 §4 row SP-Dispatch (updated) | As H15-X64; aarch64 rule unchanged | **change taken**: on x64 the old gate compared threaded code with threaded code. Open before M0 exit (H15-Q3): the cgoto.c compiler differs per target; pin it, then repeat Win64 on EPYC 7763 (loop kernel 1.591x cgoto.c, run void) |
| PERF-NOISE (amended) | Class S on a hosted x64 runner: ≥ 31 rounds, the result names the CPU model and holds for that model only. `sp-dispatch bench` applies the rule: paired ratios per round, median, order-statistic interval, void if a kernel IQR > 5% | medium | Hosted x64 labels gave 5 CPU models in 16 jobs; verdict 0.77-1.13 | A hosted label pins one CPU model, or an x64 gate host exists | **change taken**. The text of 10-quality §3 row S is not edited yet (that file is open in another workflow): add "names its CPU model; holds for that model only" |
| SPEED-CI | Hosted CI: structural gates block. Speed gates never block (`SP_SPEED=report`, 31 rounds): a `::warning` for each speed gate that does not pass, every gate in the job summary, all samples in the artifact `sp-dispatch-gate-<target>` (`bench.json`) | medium | `.github/workflows/ci.yml` spikes job | A gate host joins CI (the `perf` group): its speed gates then block | New. Hosted results stay visible; they do not hide behind a green check |
| D21-UNIT | "18 sibling-call dispatches" (D21, Spike B) means 18 dispatches of the release expansion on the SP-Dispatch loop kernel, on the same host and in the same session. The number 18 does not change | medium | H15 ADR | Spike B design review | **clarify**. Text in 04 §4, 09 §4 and 11 D21 still says "sibling-call"; edit with the next touch of those rows |
| REMOTE-CI (note) | The user allowed commits and pushes to the private repo on 2026-10-20/21. The workflow task text quotes the user: "You are free to create repo, keep it private first"; "Keep working on our plan" (workflow script text; the H15 agent did not see the user messages). This settles the REMOTE-CI critic note "commit and push only on an explicit user instruction". The repo stays private; no force-push to `main` | high | user messages as quoted by the workflow task | The user withdraws the permission or asks for a public repo | Records the instruction that the §5.4 REMOTE-CI row asks for |

**Scratch evidence.** CI logs on GitHub expire. The numbers that the decisions use, with run and job URLs, are in the measurements file. The diag branch `diag/dispatch-x64` keeps the probe code; `diag/h15-gate` ran the new gate on 4 hosted targets before the change goes to `main` (runs [39472609604](https://github.com/Brooooooklyn/ep9/actions/runs/39472609604), [39474039711](https://github.com/Brooooooklyn/ep9/actions/runs/39474039711): every job passed).
