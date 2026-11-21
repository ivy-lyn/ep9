# Why M2 is stuck: the ranked causes, the lever ledger, the gate and process costs, and the experiments X1 and X2

## Run context

| Item | Value |
|---|---|
| Date | 2026-11-21 |
| Round | The "why is ep9 stuck at M2" research: 5 angle reports (A: the x86 TS gap, B: why the jit build gives no gain, C: the lever ledger, D: the gate and the meter, E: process time), 2 adversarial verifiers, a completeness critic, then 2 experiments (X1 bytes vs placement, X2 dynamic counts) and an adversarial verifier of the experiments. Orchestrator log rows M6-ORCH-1260 (the order), M6-ORCH-1261 (the result and the decisions), M6-ORCH-1262 (the experiments) |
| ep9 tree | tag `m10-p5`, 591064fc (`591064fc07b4a50e5002baa062a461b6bece3f99`). Every ep9 code, ADR and plan line cite in this report is at this tag. To read one later, use `git show m10-p5:<path>` |
| ep9 docs | DL rows are cited by id. The month-end Path B result is not in the DL yet (DL §20.8 is in progress), so its numbers cite the scratchpad compare file (PBe) |
| x86 readings | (1) The month-end Path B on `m10-p5`: 2 Cloudflare boxes, AMD EPYC 9684X (Zen 4), 4 vCPU on 2 physical cores (SMT on), 31 paired rounds per build. (2) The mid Path B on 2856a698 (DL M10-P2-CLOSE-3) and the month-9 Path B on 86e39c65 (DL M9-END-D5), same box type. (3) X86-TOPDOWN-READ: 1 box, `m9-p5` code 86e39c65, perf stat + top-down level 1 + perf record (DL M10-P0-WPRE-1). (4) The GitHub exit-input legs on `m10-p5`: Linux l1 (EPYC 9V74), l2 (9V45), Windows w1 (7763), w2 and w3 (9V74), w4 (9V45) |
| Mac reading | The critic's own short runs on Mac aarch64 (`m10-p5` binaries and the pinned node 24.22 darwin-arm64 copy), < 60 s each, load 13 (provisional) |
| Experiments | X1: 6 cross builds for `x86_64-unknown-linux-gnu` (cargo-zigbuild 0.21.1, zig 0.17.1, rustc 1.99.1), static reads only (llvm-nm, llvm-objdump, llvm-readelf from LLVM 23.1.6); 0 x86 binary runs. X2: a temporary opcode counter build on Mac aarch64 (a patch that was never committed); 2 jitless runs and 1 jit run of the TS workload. Both: 0 boxes, scratch worktrees removed after the run |
| Upstream | `v8/v8@932ddd7ca` (`932ddd7ca4c885883ee964168004437f78858380`, local clone, commit of 2026-11-19); node v24.22.0 with V8 13.6.233.17-node.57 on every box (TD). The cited V8 lines are at this HEAD clone; the text was not checked on 13.6 |
| Method | Read only for the research; the experiments as above. Derived numbers show their inputs. "est." marks an estimate |

Short forms:
- DL = [docs/decisions-log.md](../decisions-log.md). `09:66` = `docs/plan/09-milestones.md` line 66. `Hnn` = `docs/adr/Hnn-*.md`. R21 = [21-m2-x86-ts-time.md](21-m2-x86-ts-time.md).
- S = the orchestrator scratchpad. `S/m2/...` names a file in it. These files are not in the repo; the numbers that this report uses are in [artifacts/m2-stuck/findings.json](artifacts/m2-stuck/README.md).
- OL = the orchestrator log (`S/m1m6/decisions.md`), cited by row id (M6-ORCH-n).
- PBe = the month-end Path B compare (`S/m2/pathb-m10-end/compare.txt`); PBe:29 = its line 29.
- TD = the X86-TOPDOWN-READ summary (`S/m2/m10/x86-topdown/out-1/summary.txt`; DL M10-P0-WPRE-1). TD is `m9-p5` code on 1 box.
- P15 = plan v15 (`S/m2/plan-v15.md`); P15:349 = its line 349. U = the token and run log `S/m2/usage.tsv`.
- A..E = the angle reports; VE = the engine verifier; VL = the ledger, gate and process verifier; XV = the experiment verifier. All are in `S/m2/research-stuck/`. "VE F5" = finding F5 of VE.
- Marks: **M** = measured (a file or a command). **I** = inferred. **G** = guess.
- Ratio = node --jitless time / ep9 time, paired per round (above 1.0 = ep9 is faster). Item 3 on Linux x64 needs the interval low end ≥ 1.0 in both builds (09:104-113).
- Box-neutral time = ep9 time / QuickJS time in the same rounds (lower = faster). It cancels a change of box speed.
- "sib" = the handler table of the jit build (records the profile); "ics" = the handler table of the jitless build.

---

## 0. Verdict

```
 what keeps exit item 3 closed on x86, as a share of the jit need (x1.143 at the low end, log terms)
 1 char = 2.5%

 C1 engine: x86 IPC gap at equal instructions   |████████████████████████   60%  = the whole jitless need (x1.085)
 C2 jit binary vs jitless binary at LEVER-13    |████████████████           40%  jit cell only; not codegen (X1)

 why C1 and C2 did not shrink                    C3 the levers kept the Ir meter after the instruction gap closed
                                                 C4 the time meter is too coarse (1.9-6.3%) and rationed (3 A/B a month)
                                                 C5 the phase template takes 45-50% of the tokens
 after the engine passes                         C6 the procedure tail: est. 3.5-4.5 days (Mac windows, a 66 h fuzz batch)

 where ep9 is behind node on x86 (m9-p5, cycles)  named load +0.89 G | dead statement Ldar +0.73 G | Wide step +0.59 G
 what blocks the fix                              G8 freeze: H24 R4 / rule O9, the H15 core, the handler ABI (I)
```

1. **C1, engine, about 60% of the need.** On x86, ep9 T0 ran 9.4% (jit) and 10.6% (jitless) more cycles than node --jitless at equal instructions (57.24 G and 56.67 G against 57.26 G; IPC 2.462 and 2.407 against 2.688; TD, `m9-p5`, 1 box). At `m10-p5` no counters exist. The cycle gap is now about 6% (jitless 0.942) at about 3.6% fewer instructions (x64 TS Ir 56.75 G to 54.69 G), so the CPI gap is still about 10% (I). The need depends on the host: the Cloudflare 9684X box is the most friendly x86 host for jitless (0.942); the GitHub x86 hosts read jitless 0.858-0.917 (§2.5).
2. **C2, engine and build, about 40%, the jit cell only.** At the LEVER-13 merge the jit binary became 5.5% (box 1: t(jitless)/t(jit) 1.006 to 0.951) and 6.1% (box 2: 1.004 to 0.943) slower than the jitless binary in the same rounds; the 2-box mean is 5.8%. The sign flips by host (GitHub 9V74 Linux: jit 4.6% faster). X1 killed the codegen arm: both builds run the same handler bytes, and LEVER-13 changed them in the same way. Placement is the only static difference left. It is the lead hypothesis, not a proven cause (§9.1).
3. **C3, process: the wrong meter.** The levers kept x64 Ir as their meter after the instruction gap had closed (ep9 +10.8 G Ir over node at `m7-p5`, 0 at `m9-p5`). From the month-8 end to the month-10 end, x64 TS Ir fell 9.6%. Box-neutral time moved -0.7% (jit) and -8.3% (jitless) (§3.1).
4. **C4, process: the time meter is too coarse and rationed.** One-box intervals are 1.9-6.3% wide; levers are 0.2-2%. At most 3 lever A/B boxes per month. 1 of 5 merged month-10 levers had a time read. LEVER-13 merged with no read. REG-SLOT (-7.9% x64 TS Ir) waits (§3.2).
5. **C5, process: template overhead.** Records and plans took 45-50% of the month-10 tokens. M10-P4 and M10-P5 merged 0 code. Tokens per 1% of box-neutral time rose 3.8 M (month 8) to 17 M (month 9) to 24 M (month 10) (§5).
6. **C6, the procedure tail.** 14 of 17 open gate cells are open by procedure, not by engine. Only 1 of 9 Mac windows was VALID. The counted Fuzzilli batch needs about 66 h wall on 2 boxes. The tail after the engine passes is est. 3.5-4.5 days (§4).
7. **Not causes:** a missing JIT (no JIT code runs in M2, by plan, 09:36), the workload and harness, and the build flags (ep9 has fat LTO; the darwin-arm64 copy of the pinned node has no LTO and no PGO; the linux-x64 copy is not read, I) (§2.2, §2.5).
8. **The cycle map against node points at frozen parts.** At `m9-p5`, ep9 is behind node in the named load (+0.89 G), the dead statement-position Ldar (+0.73 G, kept by H24 R4 and rule O9) and the generic Wide step (+0.59 G, an H15 core part). ep9 is already cheaper in ToBoolean (-0.07 G) and calls (-0.83 G). G8 ("no contract rewrite after M1") gave month 10 a budget of +2; 0 were used (§2.4).
9. **X2 sized the two bytecode levers.** Dead statement-position Ldar: 151,968,271 runs on TS (kill line 100 M: KEEP; est. 2.2% of x64 TS Ir, about 0.57 G cycles). A front-end feedback-slot order: 24,468,058 prefixed ops left at best (kill line 10 M: DROP); 51.1% of the prefixed ops are jump offsets (§9.2).
10. **The rate.** At the month-10 rate the jitless cell passes in about 1.45 plan months; the jit cell never passes (x0.973 per month). Item 3 needs both builds, so the jit cell sets the date (§3.3). Exit needs one of two owner-level changes: open the frozen parts (each with its own x86 time read), or change the x86 TS gate (options W or M, §7.1). The orchestrator decided to keep the gate as written for now (§8, D1).

---

## 1. The block in one diagram

```
 EXIT ITEM (09:104-113)          BLOCKING CELL at m10-p5                    CAUSE (rank)
 ──────────────────────          ───────────────────────                    ────────────
 3  TS >= 1.0x node --jitless ─┬─ linux-x64 jitless 0.942 [0.923-0.955] ──► C1 x86 IPC gap, equal instructions
    (Linux x64 rule)           │  (need x1.085)                     │        (+9.4% jit / +10.6% jitless cycles, m9-p5)
                               └─ linux-x64 jit     0.882 [0.875-0.893] ─┤
                                  (need x1.143)                          └─► C2 jit binary -5.8% vs jitless
 10  gates 1-3, profiling on ──── copies item 3 jit                               after LEVER-13 (host dependent)
 D11 gates 1-3, jitless ───────── copies item 3 jitless
                                                                     why C1/C2 stay:
                                                                     C3 Ir meter after the instr gap closed
                                                                     C4 time A/B too coarse + count cap
                                                                     C5 phase template, records, reviews
 1,2,3,4,6,10,D11 Mac a64 ─────── OPEN by procedure ───────────────► C6 busy shared Mac: 1 of 9 VALID
 1,2 win-x64 ──────────────────── "no interval" (1 kernel IQR >5%) ─► C6 no void-leg rerun step
 8 Fuzzilli 500 CPU-h ─────────── 0 counted; runs after item 3 ────► C6 66 h serial batch
 (5, 7, 9: PASS everywhere)
```

Sources: PBe:27-30, :83-86; D §1 (VL D1, D6 confirm); VL D3, D7.

---

## 2. The engine causes

### 2.1 Cause C1: the TS gap on x86 is IPC, at equal instructions

**Measured** (TD part A, `m9-p5`, one Zen 4 box; VE F5, B5):

| Engine | Instructions | Cycles | IPC | Wall ms |
|---|---|---|---|---|
| ep9 jit | 57.24 G | 23.30 G | 2.462 | 6234 |
| ep9 jitless | 56.67 G | 23.56 G | 2.407 | 6331 |
| node --jitless 24.22 | 57.26 G | 21.30 G | 2.688 | 5640 |

- Instructions are equal (0.9997). Cycles are +9.4% (jit) and +10.6% (jitless).
- The slot gap, jitless against node (top-down level 1, 6 slots per cycle; `td/L1.*.human.txt`): backend +6.45 G (about 49%), frontend +5.11 G (about 39%), SMT +0.85 G, retiring -0.8 G, bad speculation -0.4 G. The n=4 medians (TD summary:26-30) give backend +6.1 G, frontend +4.9 G. So about half backend and about 40% frontend.
- L1i misses per 1k instructions: 0.284-0.318 against 0.092. Branch misses: 1.52-1.59 against 1.25 (TD; VE F5). The extra L1i and branch misses explain about 0.5 G of the 2.26 G cycle gap (est., VE B5). The frontend part (0.8-0.85 G cycles) is larger, so a part of it is op-cache or decode bandwidth (I; the microVM has no level-2 top-down, TD summary:31).
- The L1i rate does not track time: the 64 B aligned jit build had +11.5% L1i misses and was 1.2% faster (VE F5).
- Limits: each top-down run is n=1, at 50% counter multiplexing, on a shared SMT VM (VE F5). The data is `m9-p5` code; `m10-p5` has no counters.

**Why the Mac passes and x86 does not (M, arithmetic; VE F1 corrected):**

```
                     instructions ep9 / V8      what it does
 Mac a64             50.3-50.7 G / 59.85 G      ep9 does ~16% FEWER  -> hides ep9's 7% lower IPC (4.51 vs 4.85)
 x86-64              57.24 G / 57.26 G          equal               -> the IPC gap shows in full
 ep9 x64/a64 Ir 1.094-1.096;  V8 x64/a64 0.957  -> about x1.147 of the "x86 penalty" is ep9 losing its a64 lead
```

- The inputs mix tools (Linux arm64 cachegrind, macOS `time -l`, Linux perf) (VE F1).
- This explains WHY x86 fails. It does NOT show that x64 Ir cuts turn into x86 time: LEVER-12 cut x64 TS Ir 2.1% and gave TS wall 1.000 [0.982-1.014] (DL M10-P1-B1).
- The penalty is general, not TS-only (VL D2). Mac ratio / x86 ratio: v8-v7 1.18 / 1.20, TS 1.16 / 1.19. v8-v7 passes on x86 only because it has 32% Mac headroom; TS has 5-6% (D §3.4).

**Code facts that fit C1:**

| Item | Fact | Size | Status | Source |
|---|---|---|---|---|
| Statement-position Ldar | The static counts are not like-for-like: the ep9 dump holds all 1,815 functions of the unit, the node dump only the 1,036 functions that V8 compiled (lazy). Cycle bucket (TD perf record): Ldar handler 0.985 G (ep9 jitless) against 0.253 G (node) = +0.73 G, about 1/3 of the 2.26 G gap. Static: 9,624 `S> Ldar`, 4,855 of them `S> Ldar <this>`, 3,677 of those followed by `GetNamedProperty <this>`. V8 elides such a load and moves the statement position to the next bytecode (`v8/v8@932ddd7ca src/interpreter/bytecode-array-builder.cc:753-764`). The DEAD-LDAR probe kept them on purpose (rule O9, `crates/ep9_frontend/src/bytecodegen/regopt.rs:27`; DL M10-P0-DEADLDAR) | Dynamic: 151,968,271 dead `S> Ldar` runs, est. 2.2% of x64 TS Ir (X2, §9.2) | M | VE F4; X2 |
| x64 handler code | A longer register decode (4 instructions), push/pop frames in the call and store handlers on x64 only, Wide out of line on x64 (`m8-p5` rlib disassembly) | REG-SLOT cut x64 TS Ir 7.92% / 8.01% against a64 5.20% / 5.28% (DL M10-P4-LEVER14) | static facts M; time UNPROVEN | VE F2 |
| Wide / prefixed path | ep9 runs every prefixed op in one generic out-of-line `scaled::step<S>`, a `match` over all opcodes (`crates/ep9_vm/src/interp/mod.rs:595-599`, `crates/ep9_vm/src/interp/dsl.rs:1242-1269`), plus `Wide_cold`: 5.36% = 1.26 G. node has a handler per opcode and scale: 3.18% = 0.68 G. Gap +0.59 G, about 1/4 of the cycle gap. Plan v15 rules its per-scale form "an H15 core rewrite: a DL row and stop" (P15:349) | about 2.5% of ep9 time (I, bucket). 41,058,840 prefixed runs; 45.0% involve a feedback slot, 51.1% a jump offset (X2) | M (bucket and count); fix UNPROVEN | critic; X2 |
| Feedback vector | 4 dependent loads in ep9 against 1 frame-slot load in V8 (`v8/v8@932ddd7ca src/interpreter/interpreter-assembler.cc:820-823`) | not measured | code fact; time UNPROVEN | VE F6 |
| ToBoolean of objects | `fast_truthy` covers only true, false, undefined, null and int32 (`crates/ep9_vm/src/interp/ops.rs:124-140`). But ep9 JumpIfToBoolean* + `to_boolean` = 2.23% = 0.525 G, node JumpIfToBoolean* = 2.77% = 0.590 G (TD `rec/*.sym.tsv`) | ep9 is already cheaper: a gain past parity only | M (bucket) | VE F8; critic |
| Calls | ep9 call handlers + slow call + Return + Construct about 13.8% = 3.25 G; node InterpreterEntryTrampoline 10.36% + Call builtins + call handlers about 19.1% = 4.07 G. `call_property<1>` alone is 1.16% | ep9 about 0.8 G cheaper | M (bucket) | VE F8; critic |
| Source hash | `sha256::update` 0.27% of cycles | upper bound 0.27% | M | VE F8 |
| Net cycle ledger against V8 | +9.6% of ep9 time net. The per-bucket split is biased by the V8 Star lookahead (`v8/v8@932ddd7ca src/interpreter/bytecodes.cc:261-310`) | do not rank work by bucket alone | CORRECTED | VE F3 |

### 2.2 "The JIT gives no gain on TS" is the wrong question

**Measured in code (VE B1 confirmed):**

```
 ep9 "jit" build     = T0 interpreter + Feedback::FULL (profile records) + the "sib" handler table
 ep9 "jitless" build = T0 interpreter + Feedback::ICS_ONLY               + the "ics" handler table
 No code is generated at run time in either build.
```

- `crates/ep9_vm/src/exec.rs:126-140` (the 2 presets), `crates/ep9_vm/src/interp/mod.rs:117-123` (the table pick). `ep9_masm` is in no other Cargo.toml (VE B1).
- The plan forbids a JIT that runs JS before the M2 gate (09:4, 09:36). T1 is M5 (09:57). Item 10 is "Gates 1-3 hold with all H16/H29 profiling on" (09:113).
- The comparator has no baseline tier either: `--jitless` turns off Sparkplug (`v8/v8@932ddd7ca src/flags/flag-definitions.h:998`).
- The jit build pays +1.13% x64 TS Ir for the record hooks (54.69 G against 54.08 G) and +1.0% real x86 instructions (57.24 G against 56.67 G). On richards it is +3.1% (VE B4). So by design t(jit) ≥ t(jitless) by about 1%. The measured same-round range is 0.943-1.019 (VE §1.1): other effects dominate.
- Each build runs its own copy of GetNamedProperty, SetNamedProperty and the Call handlers (about 40% of TS cycles). The linker already folds 37 of 284 handlers, Ldar and Star among them (VE B9).
- Do not use the perf "no-symbol / JIT dso" bucket (0.9% jit, 1.1% jitless) as evidence of JIT code: node --jitless shows 0.7% there (VE B1, F7).

### 2.3 Cause C2: the jit fall in month 10

**Measured (VE B2; VL §1, §2; the critic recomputed from `out-*/run-*.json`):**

```
 same-round t(jitless)/t(jit), x86 Path B, 2 boxes   (< 1 = jit slower)
                m9 86e39c65    mid 2856a698    end 591064fc
 TS             1.017 / 1.019  1.006 / 1.004   0.951 / 0.943
 richards       1.023 / 0.999  1.031 / 1.052   0.906 / 0.892
 geomean (16)   1.009 / 1.011  0.995 / 0.999   0.953 / 0.952
```

- Box-neutral (ep9 / QuickJS, same rounds), 2-box mean: jit 0.639 (mid) to 0.667 (end); jitless 0.641 to 0.630; node/QuickJS stays at 0.593-0.598 (VL §1). So the box speed change (node -8.8%, QuickJS -8.9%) cancels in the paired ratio. The jit move is an ep9 binary effect, not a box move (VL D4).
- The only code change from mid to end is LEVER-13: `crates/ep9_vm/src/call.rs`, `crates/ep9_vm/src/frame.rs` and a test file (`git diff --stat 2856a698 591064fc -- crates Cargo.toml Cargo.lock .cargo rust-toolchain.toml`).
- Its Ir change is equal in both builds: TS -0.609% / -0.614%, richards -0.606% / -0.623% (B §2.4; DL M10-P3-CLOSE-2).
- **The sign depends on the host (M; VL §2):**

```
 TS: shift of the jit build vs the jitless build, mid -> end
   CF 9684X Linux   ████████████  jit slower 5.5% (box 1; box 2 6.1%)
   GH 9V45  Win     ███████       jit slower 3.3%
   GH 7763  Win     ██████        jit slower 3%
   GH 9V74  Linux   jit FASTER 4.6%   <- opposite
```

- Other whole-binary changes also moved the two builds in opposite directions. PGO: jit 1.4% slower, jitless 1.2% faster (DL M10-P0-WPRE-2). Before LEVER-13, t(jitless)/t(jit) on TS ranged 0.977-1.017 by tree and box (VL §2).
- Builds are deterministic: the same ep9 sha256 on both boxes (PBe:21). So a layout draw is per commit. Two boxes cannot average it, and it ships with the gate binary (VE B6).
- **What X1 settled (§9.1):** not per-table codegen. The handler bytes are the same in both builds, and LEVER-13 changed them in the same way. Placement moved, and differently in the two builds. Layout is the lead hypothesis; its time effect is UNPROVEN.
- Part of the month-10 jitless gain is not explained by Ir either (+1.8-2.3% time from -0.61% Ir). Do not bank 0.942 as a floor (VE B8).

### 2.4 The cycle map against node, and the freeze

**Per-bucket cycles, jitless against node (M from the TD perf record; `m9-p5`; 1 box):**

```
 bucket (G cycles)              ep9 jitless   node     gap     note
 Ldar handler                     0.985       0.253   +0.73    V8 elides dead Ldar (bytecode-array-builder.cc:753-764)
 named load (no Wide)             7.60        6.71    +0.89    ep9: 4 dependent loads to the feedback entry (VE F6)
 Wide / prefixed path             1.26        0.68    +0.59    ep9: one generic scaled::step<S>; V8: a handler per op and scale
 Star + Mov                       0.99        0.39    +0.59    biased: V8 runs Star inside the producer (Star lookahead)
 ToBoolean jumps + to_boolean     0.53        0.59    -0.07    ep9 already cheaper
 calls (handlers, slow, Return)   3.25        4.07    -0.83    node InterpreterEntryTrampoline 10.36%
 GC-like (rough)                  ~1.6        ~1.3    ~+0.3
 total cycles                    23.56       21.30    +2.26
```

- Use this as a map, not a ranking (VE F3: the Star lookahead moves cost between buckets). The positive rows sum to more than the gap; calls offset them.

**The freeze is a cause, not only a cost (I, from plan and code facts):**

```
 bucket where ep9 is behind      the fix it needs                   what blocks it
 Ldar +0.73 G                    elide statement-position Ldar       rule O9 / H24 R4 (Frozen): DEAD-LDAR kept them; G8 +1 (P15:230)
 Wide +0.59 G                    per-scale handlers                  "an H15 core rewrite: a DL row and stop" (P15:349)
 x64 decode + frames (REG-SLOT)  operand encoding, ABI               G8 +2, a product call, waits for month 11 (DL M10-P4-WPRE-3)
 IPC as a whole                  pinned registers / generated         H15 dispatch core and handler ABI Frozen since M1 exit (H15:5)
                                 handlers (the V8, JSC form)
```

- G8 = "No contract rewrite after M1" (`docs/plan/02-architecture.md:26`). Month 10: budget +2, used 0 (DL M10-P5-G8).
- So the levers that the plan allowed (in-body changes, judged by x64 Ir) could not reach the parts where the measured gap is. This explains "the levers are too small" better than "the meter was wrong" alone.

### 2.5 Not causes: the workload, the harness, the build flags, the node build

**Workload and harness: fair (M).** One file with the same sha256 for all engines (PBe:19-20); one process per run, N = 5 (the benchmark's own), no warm-up, wall time; 31 paired rounds, rotated; net = run minus the empty script of the same round. Engine flags: ep9 none, node only `--jitless`. No heap or GC flag on either side.

| Fact | ep9 | node --jitless | Source |
|---|---|---|---|
| CPU / wall on TS (Path B end, 4 cells) | 1.024-1.028 | 1.043 | `M.cpu.rows` of the Path B run JSON |
| fixed cost (N=1 minus one iteration), Mac a64, 3 reps | jitless 74 ms (2.9% of N=5), jit 93 ms | 91 ms (3.2%) | critic runs |
| per iteration, Mac a64 | jitless 501 ms, jit 513 ms | 548 ms | same |
| GC-like symbols, x86 TD (rough regex) | about 6.7% of cycles | about 6.0% | TD `rec/*.sym.tsv` |

So helper threads, start-up, parse and compile are not the gap. GC is at most about 0.3 G of 2.26 G cycles (I).

**Build flags already favour ep9 (M).**

| | ep9 (Path B build) | node 24.22 (pinned) |
|---|---|---|
| LTO | fat, codegen-units 1 (`Cargo.toml:87-91`) | off (`process.config` of the darwin-arm64 copy) |
| PGO | off (PGO-PROBE failed, DL M10-P0-WPRE-2) | off |
| target-cpu | x86-64 baseline (no RUSTFLAGS) | generic |
| pointer compression, sandbox, external code space | n/a (NaN-boxed values) | 0, 0, 0 |
| BOLT / symbol order | never tried | n/a |

- V8 applies its x64 builtins PGO profile only with pointer compression, the external code space and the sandbox on (`v8/v8@932ddd7ca BUILD.gn:870-878`); the pinned node has all 3 off. The linux-x64 copy was not read; the same release config is likely (I).
- BOLT or a symbol order file is the untried standard tool for a frontend-bound interpreter and for a per-commit layout draw (C2).

**The box is not the exit host, and the need depends on the host (M).**
- All TD, A/B and Path B boxes are Cloudflare standard-4, EPYC 9684X, 4 vCPU on 2 physical cores (SMT on; `S/m2/pathb-m10-end/out-1/box-facts.txt:45-48`). The gate host rule asks for bare metal with the SMT sibling off and turbo off (`docs/plan/10-quality.md:151`).
- The owner action names an EPYC 7763 (Zen 3) gate host. On that CPU the H15 "x86-64 Zen 3 condition" is still open and freezes at M2 exit (H15:7, :126). A Cloudflare pass reads "MARGINAL (no gate host)", but a later fail on the gate host reopens M2 (`docs/plan/10-quality.md:158`).
- Same CPU model, another box: TS jit 6.23 s at IPC 2.46 (TD) against 8.82 s at IPC 1.89 (`S/m2/m10/p1/x86-ab/out-1/summary.txt:43`) at equal instructions. Only same-round ratios carry.

TS against node --jitless at `m10-p5` on every x86 host read (PBe:29, :85; `S/m2/gw12/out/591064fc07b4/winpathb.log:38-63`):

| Host | CPU | OS | jit | jitless | jitless need at the median |
|---|---|---|---|---|---|
| CF box 1, 2 | 9684X (Zen 4) | Linux | 0.882 / 0.885 | 0.942 / 0.942 | x1.06 |
| GH l1 | 9V74 (Zen 4) | Linux | 0.914 | 0.899 | x1.11 |
| GH l2 | 9V45 (Zen 5) | Linux | 0.865 | 0.876 | x1.14 |
| GH w1 | 7763 (Zen 3) | Windows | 0.887 | 0.917 | x1.09 |
| GH w2, w3 | 9V74 (Zen 4) | Windows | 0.894 / 0.893 | 0.907 / 0.905 | x1.10 |
| GH w4 | 9V45 (Zen 5) | Windows | 0.823 | 0.858 | x1.17 |

The CPU generations are inferred from the model names. So the x1.085 need of §0 is the best case; the full range is x1.06-1.17 (jitless) and x1.09-1.22 (jit).

---

## 3. The lever ledger and the rate

### 3.1 Month totals (box-neutral x86 TS time = ep9 / QuickJS, same rounds, 2-box mean)

| Reading | jit/qjs | jitless/qjs | x64 TS Ir jit | Month Ir | Month time jit / jitless | Time per 1% Ir (jit / jitless) |
|---|---|---|---|---|---|---|
| m7 end | 0.779 | 0.772 | 67.96 G | - | - | - |
| m8 end | 0.672 | 0.687 | 60.53 G | -10.9% | -13.7% / -11.0% | 1.26 / - (no m7 jitless Ir) |
| m9 end | 0.656 | 0.666 | 56.75 G | -6.2% | -2.4% / -3.1% | 0.39 / 0.49 |
| m10 mid | 0.639 | 0.641 | 55.03 G | - | - | - |
| m10 end | 0.667 | 0.630 | 54.69 G | -3.6% | **+1.7%** / -5.4% | **-0.46** / 1.46 |
| target | node/qjs 0.597 | 0.597 | - | - | - | - |

Sources: VL §1 (`V-work/series.py`, per-round medians of the Path B run JSON); Ir from C §1.2 and R21 §2.1, §9.3. "Time per 1% Ir" is derived.
Gap left at the median: jit 0.667 / 0.597 = x1.117; jitless 0.630 / 0.597 = x1.055 (derived).

```
 x86 TS time, box-neutral (lower = faster)
 jit      0.779 ─► 0.672 ─► 0.656 ─► 0.639 ─► 0.667     m8 -> end: -0.7%
 jitless  0.772 ─► 0.687 ─► 0.666 ─► 0.641 ─► 0.630     m8 -> end: -8.3%
 x64 Ir   67.96 ─► 60.53 ─► 56.75 ─► 55.03 ─► 54.69 G   m8 -> end: -9.6%
          m7       m8       m9       mid      end
```

- Month 8 was right to target Ir: at `m7-p5` ep9 ran +10.8 G Ir over node on TS (OL M6-ORCH-469). At `m9-p5` the counts are equal (TD). After that, Ir stopped being the right meter (VL C1, C3).

### 3.2 Month 10 per lever (x64 TS Ir; x86 time = a one-box A/B, B/A wall)

| Lever | Ir jit / jitless | x86 time read | Merged | What decided | DL |
|---|---|---|---|---|---|
| PREALLOC + BESTFIT (owner order) | -0.40% / -0.40% (PREALLOC -0.217% / -0.219%, BESTFIT -0.178% / -0.179%) | none | yes, M10-P0 | Ir floor | M10-P0-BASE; OL M6-ORCH-1032 |
| PGO-PROBE | instructions +2.6% | jit 1.4% slower, jitless 1.2% faster | no | the +3% line failed | M10-P0-WPRE-2 |
| DEAD-LDAR | -0.67% / -0.68% | none | no | kill line -1.0%; G8 +1 | M10-P0-DEADLDAR |
| LEVER-12 Q1 NAMED-MISS | -2.10% / -2.17% | jit 1.000 [0.982-1.014], jitless 0.995 [0.969-1.029] | yes, M10-P1 | Ir line | M10-P1-B1 |
| X86-ALIGN | 0 | 0.988 [0.977-0.997] (TD box); 0.995 / 0.987 [0.943-1.006] (P1 box) | no | the X86T time rule | M10-P1-X86ALIGN |
| STAR-LA2 | 0 | jit 0.994 [0.983-1.007], jitless 0.968 [0.958-0.977], t0-branchy +5.4% / +6.5% | no | X86T: jit straddles 1.0, the t0-branchy row | M10-P3-STARLA2 |
| Q1b NAMED-STORE-MEGA | -0.55% / -0.57% | none | yes, M10-P2 | Ir only | M10-P2-CLOSE-2 |
| LEVER-13 CALL-FILL (x86 only) | -0.61% / -0.61% | **none** | yes, M10-P3 | Ir only; the W-pre gave the last A/B box to STAR-LA2 (VL D5) | M10-P3-CLOSE-2 |
| LEVER-14 REG-SLOT probe | **-7.92% / -8.01%** (a64 -5.2%) | none (K2 put off) | no | the count cap 3 of 3, G8 +2, a product call | M10-P4-LEVER14, M10-P4-WPRE-2 |

Sources: C §2.3, §2.4; VL C4, D5, E7; VE F9.

- **Ir to time per lever:** LEVER-12: 0 (instructions 0.980, cycles 1.000). X86-ALIGN and STAR-LA2: 0 Ir, -0.5% to -3.2% time. PGO: +2.6% instructions, +1.4% jit time (C §3.2; VL C3).
- **The gates were asymmetric (VL C4):** Ir-only levers merged with no time read. Time-path levers needed an interval wholly below 1.0 on one box, in both builds. STAR-LA2 jitless and X86-ALIGN showed one-box gains; a pooled 2-box read would be needed to call them real.
- **Resolution:** one-box intervals are 1.9-6.3% wide; levers are 0.2-2%. Cycle intervals are only 0.59-0.79 times the wall width (VL §5).

### 3.3 At this rate, when does item 3 pass?

| Cell | Need (low end) | Month-10 rate (Path B 2-box mean) | Plan months to pass | Calendar (est.) |
|---|---|---|---|---|
| jitless | x1.085 (0.922 low end) | 0.8905 to 0.942 = x1.058 | ln 1.085 / ln 1.058 = 1.45 | months 9-10 ran 2.2-2.3 days each (E §1.1): about 3-4 days of work |
| jit | x1.143 (0.875 low end) | 0.908 to 0.8835 = x0.973 | **never** at this rate; 4.7 at the month-9 rate (x1.029); never at the months 9-10 rate (x1.001) | - |

Sources: PBe:29, :85; C §1.1, §3.5; VL C9 (the arithmetic holds; the inputs are not measured).

- Item 3 needs both builds, so **the jit cell sets the date**.
- If C2 is undone (jit back to the jitless level, as at mid), the jit cell passes with the jitless cell (I).
- The jitless rate is partly unexplained by Ir (VE B8). It may not repeat. The need is also larger on every host other than the CF box (§2.5).
- Then add the procedure tail (§4.3).

---

## 4. The gate and procedure causes

### 4.1 OPEN by engine (ENG) against OPEN by procedure (PROC), `m10-p5`

| # | Item | Mac a64 | linux-x64 (Path B) | win-x64 | linux-a64 |
|---|---|---|---|---|---|
| 1 | Octane-7 ≥ 1.33x QuickJS | PROC (jit provisional) | PASS 1.709 / 1.724, 1.795 / 1.802 | PROC: 1 kernel IQR 5.1-5.6% | N/A |
| 2 | v8-v7 ≥ 1.0x node --jitless | PROC (jit provisional) | PASS 1.068 / 1.054, 1.107 / 1.107 | PROC: crypto IQR 5.5-7.0% | N/A |
| 3 | TS | PROC: passed in GW10b (jit 2.570 s [2.552-2.630]); VOID or provisional since | **ENG**: jit 0.882 / 0.885, jitless 0.942 / 0.942 | mixed; the 9V74 pairs pass the 09:66 form | N/A |
| 4 | G7 memory | PROC: block B MISSING | PASS | PASS | PROC: reader rerun |
| 5 | test262 + FASTDIFF | PASS | PASS | PASS | PASS |
| 6 | splay | PROC | PASS (low end 1.045-1.076) | PASS-side | N/A |
| 7 | JetStream list | PASS | PASS | PASS | PASS |
| 8 | Fuzzilli 500 CPU-h | PROC: 0 counted | same | same | same |
| 9 | P1, P2, P4 | PASS | PASS | PASS | PASS |
| 10 | gates 1-3, jit | PROC | **ENG** (item 3 jit) | follows 3 | N/A |
| D11 | gates 1-3, jitless | PROC | **ENG** (item 3 jitless) | follows 3 | N/A |

Sources: D §1 (VL D1, D6; jit item 1 is already AGREE above on the 9V74 pair); PBe:27-30, :83-86; GW10b: OL M6-ORCH-733.
**Count: 3 ENG cells, all one fact (x86 TS). 14 PROC cells.**

### 4.2 Why the PROC cells stay open

- **Mac windows:** since 11-06, 1 of 9 tries was fully VALID. Other work on the Mac voided 7, the orchestrator's own agents 1 (VL D3). On 2026-11-21, in the load rows 09:33-11:48 of `S/m2/gw12/load/loadlog.tsv`, 1 row has load1 ≤ 3.0 (the guard line): 11:33:02, 2.67. At 12:26 the file had 173 rows, still 1.
- The Mac "marginal -> 2x rounds" step has no command (`S/m2/gw12/run.sh:300` runs `--rounds 10`) (VL D10).
- **win-x64 items 1, 2:** "no interval" because one kernel's IQR was 5.1-7.0%. The void-leg rerun is a plan-v16 route only (VL D6, D10; DL M10-P4-WPRE-6).
- **The gate form:** the low end sits 0.8% (jit) and 2.0% (jitless) under the median. "Both builds" takes the worse of two binary draws (D §3.3).

### 4.3 Item 8 arithmetic

| Fact | Value | Source |
|---|---|---|
| rule | one counted run after the last fix, on the exit-candidate tree | `docs/plan/10-quality.md:179`; V9-D24 |
| counted batch | 16 rounds x 2 boxes x 4 workers x 4 h = 512 CPU-h | P15:574 (V15-D23) |
| box cost | 0.256 box-h per CPU-h: about 131 box-h | P15:502 |
| wall | about 66 h on 2 boxes | VL D7 |
| so far | 0 of 500 counted; 96 CPU-h counted false since month 7; 0 Fuzzilli crashes | `S/m2/fuzz/ledger.md` |
| monthly batch | 32 CPU-h counted false, 8.2 box-h | P15:470-471 |
| if the monthly batches counted | 500 / 32 = 15.6 months | derived |
| order | runs only after every x86 speed cell is AGREE above | P15:491-497 |

- Item 8 does not block today. It is the longest serial step after item 3.
- The tail after item 3 passes (est., from plan v15 §13.5): tag CI about 1.7 h + Path B about 1 h + exit inputs about 2.7 h + 1-2 Mac nights + the 66 h batch = **about 3.5-4.5 days**, if no crash and the Mac is quiet.
- The batch is 16 times longer than any M2 Fuzzilli campaign (32 CPU-h). cargo-fuzz (counted false) found crashes in 3 M2 GitHub runs: the regexp target at `m8-p1` (3), the regexp and unit_loader targets at `m8-p5` (1 each), the regexp target at `m9-p2` (1) (`S/m2/fuzz/ledger.md:17, :20, :25`). Fuzzilli reaches the regexp engine too. So a stop of the counted batch has a real chance (G on its size).

---

## 5. The process causes

### 5.1 Hours and tokens

| Month | Wall | Tags | Workflow / agent runs | Tokens | Box-neutral time jit / jitless | Tokens per 1% time |
|---|---|---|---|---|---|---|
| 7 | 124.8 h | 6 | ~123 workflows (no usage log) | not logged | - | - |
| 8 | 112.8 h | 6 | 110 / 275 | 46.83 M | -13.7% / -11.0% | 3.8 M |
| 9 | 56.2 h | 6 | 86 / 236 | 46.27 M | -2.4% / -3.1% | 17 M |
| 10 | 53.5 h | 6 | 103 / 296 | 43.24 M | +1.7% / -5.4% | 24 M |

Sources: E §1.1 (git tag times, U); VL §4 (month 8 holds 9.58 M in outcome text; its agent column sums to 268, not 275: open, §11); VL §1.

**Month-10 tokens by kind** (E §1.2; the shares depend on the classifier; records + plan are 45-50% either way, VL E1):

```
 records (DL, H16 records, MODES-CHECK, fact checks)  ████████████ 10.22 M  24%
 plan / W-pre / briefs                                ███████████   9.48 M  22%
 engine code (role builds)                            ██████████    8.98 M  21%
 reviews + fix rounds                                 ██████████    8.69 M  20%
 infra, owner orders, research, measurement           ██████        5.89 M  14%
 item-3 speed work, all kinds                         about 11.9 M  27.6%
```

**Month-10 wall, 53.5 h** (E §0; the union of agent intervals, ±1 h):

```
 any item-3 agent running   ██████████████████████  22.6 h  42%
 item-3 code + review       █████████████████       16.9 h  32%
 no agent running at all    ███████████             11.3 h  21%
 last DL commit -> tag      ████████████████        15.97 h  (overlaps the next W-pre; not idle; VL §4)
```

### 5.2 The process cost table

| Cause | Measured cost | Source |
|---|---|---|
| a fixed 6 phases per month | M10-P4 and M10-P5 merged 0 code: 38 runs, 6.9 M tokens (16%). P15:361 already said they add about 0 to item 3. P4 did produce the REG-SLOT probe off the main line | E §1.3, §2; VL E2 |
| a tag CI per phase | 6 x 101 min, about 1,950 runner-min each; no code defect found by a tag CI in months 8-10 | E §1.5; VL E3 |
| MODES-CHECK at every tag | 6 checks in 2.2 days = 19 times the weekly rate of the owner rule | VL E9 |
| docs-only review loops | MODES-TEXT: 6 fix workflows, 12 runs, 1.22 M tokens, 10:29-16:11 | U:214-225; VL E8 |
| box use, month 10 | fuzz counted false 9.63 box-h (61%), Path B 3.76 (24%), lever A/B 1.40 (9%), TD + PGO 1.05 (7%) | E §1.7; VL E6 |
| the box budget shape | 15.9 box-h already held the month-end lines; the lever A/B line was 1.0 box-h and was overspent (1.4). Real slack about 1.2-1.5 box-h | VL §3 |
| the lever A/B count cap (≤ 2, then 3) | 1 of 5 merged levers had a time read; LEVER-13 merged blind; REG-SLOT K2 moved to month 11 | C §2.3; VL D5 |
| stops | a month-8 session stop of 31.9 h (cause not logged); a host weekly-limit stop on 11-18 | OL M6-ORCH-671, -672, -946; VL E10 |
| no x86 cycle profile until day 14 | the first per-symbol x86 profile was 11-19 (X86-TOPDOWN-READ); month 9 picked STAR-FUSE by simulated Bim, 12 times the real misses | OL M6-ORCH-1028; VL C6; C §5 |

Gates that did pay (keep them): Codex rounds 1-3 on code found HASH-SEED [high], A1H-SCHEMA [high], EXIT-INPUTS-2 [high] and the LEVER-13 test defects; SIDECI found the STAR-LA2 arm64 stack overflow; W-pre round 1 found the vacuous align64 column (E §1.5).

---

## 6. What changed from R21 and earlier reports

| Earlier claim | Where | Verdict now |
|---|---|---|
| on x86 the TS gap is cycles per instruction, not instruction count | R21 §9.1 | **Right** (TD; VE F5). The split is about half backend, about 40% frontend |
| Ir is a weak x86 time signal (0.31-0.46 per 1%) | R21 §2.2, §9.3 | **Right, and stronger:** per lever the factor is 0 to 1, or negative (VE F1; VL C3) |
| the plan sums still use Ir x one factor (REG-SLOT +2.5..+5%) | R21 §9.5, plan v15 | **Wrong method.** Only a K2 read gives it (C §9) |
| "the x86 penalty did not move: 1.157 / 1.192" | R21 §2.1 | Numbers right. **Incomplete:** the same penalty holds on v8-v7 (1.18 / 1.20). It is a general x86 interpreter gap; TS only has the least Mac headroom (VL D2) |
| jit against jitless depends on the host | R21 §9.5 | **Right.** The LEVER-13 shift flips sign on GitHub 9V74 (VL §2). It also depends on the commit; both hold |
| 69.87% of the static TS `Ldar` sites are dead | R21 §5.1 | **Static only.** Dynamic: 75.9% of the Ldar runs are dead; the DEAD-LDAR probe removed only the no-mark class (46.7 M); the statement-position class (152.0 M) is 3.3 times larger (X2) |
| the feedback slot is 85.9% of the prefixed ops | WIDE-READ (DL M10-PRE-WIDE-READ) | **Static share.** Dynamic: a slot is involved in 45.0%, a jump offset in 51.1% (X2) |
| win-x64 has its own "≥ 1.0x node" need | R21 §0 | **Wrong:** 09:66 asks Windows to stay within 10% of Linux x64 (D §10) |
| "Bim is a note, not a lever reason" | R21 §3, V15-D6 | **Right** (C §5) |
| R21 ends before the month-end Path B | R21 | **Incomplete:** it does not see the jit shift after LEVER-13 (E §5) |
| PGO est. +3..+10% | plan v15, R21 §0 | **Wrong for jit:** measured jit -1.4%, jitless +1.2%, +2.6% instructions (DL M10-P0-WPRE-2) |
| the box limit = box-hours | plan v15 rule 56 | **Incomplete:** the binding limit was the A/B count cap inside a 15.9 box-h plan that was mostly pre-assigned (VL §3) |
| 6 phases as fixed process slots | plan v15 §7 | **Cost shown:** 2 phases with 0 code (E §2) |
| "the speed of the loop was good" (tags per hour) | an earlier process review | **Wrong measure:** tags per hour, not exit progress per hour (E §5) |
| "the JIT does not pay on TS" | the task framing | **Wrong question:** no JIT runs in M2 (VE B1) |
| "the Mac passes or nearly passes item 3" | the task framing, plan v15 §0.2 | The Mac **passed** in GW10b (jit 2.570 s [2.552-2.630]); it is OPEN by procedure since (D §1) |
| "node is a PGO-tuned interpreter" | the task framing | **Wrong** for the pinned 24.22 (the darwin-arm64 copy: no LTO, no thin LTO, no PGO; pointer compression, sandbox, external code space 0). The linux-x64 copy was not read (I: the same config) |
| "the levers are too small for the known need" | OL M6-ORCH-1250 to -1256 | **Incomplete:** the largest measured lever was held by process, one unread merge came with the 5.8% jit shift, and the levers that could reach the gap buckets were frozen (§2.4; E §5; VL C2) |

---

## 7. Recommendations for month 11, with kill lines

Ranked by the expected effect on the exit date. The engine rows come first: every gate form except W and M still needs jitless ≥ 1.0, which fails on all 6 x86 hosts (0.858-0.942).

| # | What | Kind | Expected effect | Cost | Risk | Kill line |
|---|---|---|---|---|---|---|
| 1 | **The layout-pin row on the first month-11 x86 box (not a lever).** One CF box, 31 rounds, TS and richards, wall + cycles + L1i, same rounds: end-jit, end-jitless, and both again with `-C llvm-args=-align-all-functions=6`; plus `nm -S` of the hot handlers of every box binary (checks the +32 B offset of X1) | measurement (the XV order; D3 put the LEVER-13 A/B first; plan v16 decides) | decides whether C2 (about 5.8% of jit time) comes back by a layout rule; makes later one-box reads honest | about 1 box-h | none (read only) | Confirm: in the aligned pair t(jitless)/t(jit) ≥ 1.00 and the aligned jit gains ≥ 3%: a handler-alignment rule goes into every A/B and the gate build. Kill: every jit variant within ±1% of end-jit: then the LEVER-13 A/B f09c2daa vs 049ed54d (both builds, plus one GitHub 9V74 Linux leg) is next |
| 2 | **If layout is confirmed: fix the hot-handler placement in both builds** (a fixed order + 64 B alignment, the kept X86-ALIGN branch, or BOLT / a symbol order file) and judge it by spread, not by mean | engine (a build rule; G8 +1 at H15:39) | removes the per-commit draw (up to 5.8% on jit, est.) | 2-4 agent-days (B §4) | may fix one host and not another (the sign depends on the host) | a 4-way link spread after the change not below the spread before |
| 3 | **REG-SLOT-ENCODE: the K2 read, then the merge if it holds** | engine (G8 +2: a product call on the G8 budget) | -7.9% x64 TS Ir; time 0..+8% (factor 0-1) | 0.5 box-h + the merge role | ABI 6 to 7, 121 goldens, the aarch64 frames row open (C §4) | TS wall median > 0.990 or the interval not wholly below 1.000, in either build (the pre-registered K2, DL M10-P4-WPRE-2) |
| 4 | **Elide the dead statement-position Ldar** and move the statement position to the next bytecode (the V8 form, `bytecode-array-builder.cc:753-764`) | engine (G8: opens H24 R4 and rule O9) | 152.0 M runs removed: est. 2.2% of x64 TS Ir, about 0.57 G cycles, 78% of the Ldar bucket gap (X2) | 1 G8 row + a role + a time A/B on the pinned layout | a debugger step position moves; Ir-to-time factor 0..1 | no kill line yet: it is pre-registered with its G8 row (the D2 meter: a 2-box pooled time A/B) |
| 5 | **Change the lever loop's meter:** every hot-handler lever gets an x86 time A/B before merge, 2 boxes pooled; drop the count cap, keep a box-h cap; pick levers from the x86 cycle profile (named load, statement Ldar, Wide), not ToBoolean or calls (ep9 is already cheaper there) | process (a part needs the owner) | turns the 0..1 Ir factor into measured time; stops blind merges | 0.5 box-h per lever | slower merges | 3 pooled reads in a row cannot resolve ±1%: the meter is still too coarse; stop and use a gate host |
| 6 | **A G8 product call that opens the frozen parts** the buckets point at, each with a time read: H24 R4 (row 4), the x64 handler ABI (row 3), the Wide path. A front-end slot order is dropped (X2); the Wide jump-offset half (51% of prefixed ops) needs a jump-form design study (branch islands or per-scale jumps), not a box | **PRODUCT CALL** | reaches the +0.73 G and +0.59 G buckets | 0 compute for the call | contract churn after M1 | - |
| 7 | **Read item 3 on the gate host model (EPYC 7763) before the exit run**: a Linux leg on that model, or a rented bare-metal 7763 | measurement / owner action | a CF pass can reopen M2 there | 1-4 runner-h, or a rental | - | jitless < 0.95 there: a CF pass is not safe |
| 8 | **Cut the template:** no fixed phase count (a phase = one lever set); no docs-only phase tag; MODES-CHECK weekly (the owner rule); W-pre 1 round + (a)/(b) only; Codex round 1 only on docs | process | about 16% of tokens (P4/P5-type phases) and 1-2 W-pre rounds per phase | 0 | a missed record defect | any code defect found only by a later full CI |
| 9 | **Shorten the post-engine tail in parallel:** test a GitHub Fuzzilli lane (1 runner-h) or allow 4 boxes for the counted batch; add the Mac 2x-rounds step and the void-leg rerun; allow Mac windows on any quiet night | process; venue and box changes are **PRODUCT CALLS** | the 66 h batch to about 33 h on 4 boxes; fewer PROC cells | 0-1 runner-h to test | a GitHub venue changes the V9-D24 form | the GitHub job cannot record CPU-h in sealed form |
| 10 | **A generated interpreter** (masm handlers, pinned registers, the V8 and JSC form) as a plan item | **PRODUCT CALL** (plan scope) | the only single item with an est. ≥ 10% ceiling (G, A §5) | 2-4 plan months (G) | a second handler set to keep equal | - |
| 11 | **A gate host (bare-metal Linux x64)** | owner action | removes the "MARGINAL (no gate host)" reopen risk; less noise | purchase or rent | may read higher or lower | - |
| 12 | **Decide how item 3 / item 10 is formed** (options in §7.1) | **PRODUCT CALL** | 0 on the exit date until the jitless cell moves (only W or M unblock now) | 0 compute; a DL row and a pre-registration before the next Path B | a line set after the data is a moved goal post unless pre-registered | - |

### 7.1 The gate options (information for the owner; the orchestrator took K, §8 D1)

| Option | What changes | Buys | Costs |
|---|---|---|---|
| K. keep as written | nothing | the hard gate stays true | jit date unknown at the month-10 rate (never); jitless about 1.5 plan months |
| W. waiver + parity debt (09 §5 form) | pre-register an x86 TS line and a root-cause ADR; the debt is due at M5 exit | M5 can start | to pass today the line must be ≤ 0.875 (jit) and ≤ 0.922 (jitless) (VL D9); jitless x86 is a shipped mode |
| W2. waive the jit cell only | keep jitless ≥ 1.0 | keeps the shipped-mode promise | still fails today (jitless 0.922) |
| R. re-form item 10 as a same-box bound | jitless must reach ≥ 1.0x node; the jit cell must hold t(jit)/t(jitless) ≤ a pre-registered bound (for example 1.03) in the same rounds | tests what item 10 says ("profiling on", 09:113); removes the per-commit draw from the absolute gate | a rule edit; the bound must be set before the next Path B |
| M. move the x86 TS half to M5 exit | the 09:106 text and §5.2 | a fixed due point | at M5 the jit build has T1, so only the jitless half keeps its meaning |

The purpose of M2 in the plan text: "No JIT tier that runs JS starts before T0 reaches V8-jitless parity" (09:4). R, W2 and K all still need jitless ≥ 1.0.

---

## 8. Decisions taken (orchestrator; DL rows land with plan v16)

These are **decisions**, not findings. The orchestrator took them under the owner order "decide yourself" (OL M6-ORCH-1261). Each one is a plan v16 input; the plan workflow writes the DL rows.

| # | Decision |
|---|---|
| D1 | **The gate stays as written (option K).** No waiver and no re-form now: every option except W and M still needs jitless ≥ 1.0, which fails on all 6 x86 hosts, so a re-form buys nothing today. Option R is read again only when jitless passes, and only as a pre-registered row. The options go to the owner as information (§7.1) |
| D2 | **Month 11 changes the meter.** Every hot-handler lever gets an x86 time A/B before merge (2 boxes pooled, cycles + wall). The A/B count cap is dropped; the box-h cap stays. Levers are picked from the x86 cycle profile (named load, statement Ldar, Wide), not from Ir |
| D3 | **Settle C2 before any new lever.** The bytes-vs-placement compare of 2856a698 and 591064fc (0 box; it ran as X1, §9.1) and a LEVER-13 A/B on the first month-11 box with REG-SLOT K2 |
| D4 | **G8: plan v16 opens the frozen parts that the buckets point at**, each with a time read: REG-SLOT (ABI 6 to 7) first, then H24 R4 statement Ldar and the Wide path |
| D5 | **The template:** a phase = one lever set; no docs-only phase tag; W-pre 1 check round + (a)/(b) only |
| D6 | **The month-end counted-false Fuzzilli batch** (8.2 box-h planned, P15:470-471; 9.63 box-h with the lost attempt = 61% of the month-10 box-h, §5.2; counts 0 for item 8) shrinks in month 11 to a short health run; its box-h go to time A/Bs; the owner rule "fuzz only at month end" holds |
| D7 | **A generated-interpreter design study** (read only) runs in month 11 as an option paper, with no build |

After X1, the experiment verifier proposed an order for the first month-11 box (XV §4): the layout-pin row of §7 row 1 before the LEVER-13 A/B, because X1 shows that LEVER-13 changed the handler bytes in the same way in both builds, so a revert acts on C2 only through placement, which the layout-pin row tests directly. This is a plan v16 input, not a decision yet.

---

## 9. The experiments (verified form)

### 9.1 X1: bytes against placement for C2 (XV verdict: CORRECTED, small)

**Verdict: NOT CODEGEN.** The codegen arm of the kill/confirm line is killed (M): at each commit the jit and the jitless binaries run the same handler code, and LEVER-13 changed it the same way in both. Layout is the only static difference left. It is the lead hypothesis for C2, not a confirmed cause: the line-count sign agrees with the measured sign only under the fitted +32 B box offset, and line count is not time.

```
 C2 (jit -5.8% vs jitless at LEVER-13)
   codegen? ── jit vs jitless handler code equal (599/599, also with the branch-target index) ──► KILLED (M)
   layout?  ── only placement differs; the sign fits only with the fitted +32 B box offset ────► lead hypothesis, UNPROVEN (time)
```

**Method.** 6 cross builds (mid 2856a698, end 591064fc and the `m9-p5` check build 86e39c65; jit = default features, jitless = `--no-default-features`), in the project cross form: `cargo zigbuild --release --locked -p ep9_cli --target x86_64-unknown-linux-gnu`. 599 symbols compared per build: every `ep9h__*` handler, the match loop, the prefixed steps and the slow call path. Each symbol is cut at its nm size and normalized (calls by symbol name, crate hashes removed, rip-relative data by name). XV re-ran the compare with a second normalizer that keeps the local branch-target index (the X1 normalizer drops branch targets, although its comment says it keeps their ordinals). The result is the same.

**Does the cross build match the box build (XV §1.1)?** For codegen, yes: rustc 1.99.1 (58cf8f9c3 2026-10-13) on the box (its toolchain log) and in the `.comment` of the cross build; the same `[profile.release]` (fat LTO, codegen-units 1, line-tables-only, panic abort; `Cargo.toml:87-97`); `.cargo/config.toml` has only `[alias]`; no RUSTFLAGS in the orchestrator env or the build pool; baseline target-cpu. The linker differs (Homebrew LLD 22.1.8 via zig, against the box default), which changes placement only.

| Compare | Symbols | Differ |
|---|---:|---:|
| mid jit against mid jitless | 599 | 0 |
| end jit against end jitless | 599 | 0 |
| mid jit against end jit | 599 | 21 |
| mid jitless against end jitless | 599 | 21 (the same 21) |

- **What LEVER-13 changed (M):** each call handler gets +1 instruction, +8 B: the register-fill loop now does 2 stores per pass (`movq $0xa,(%rsi); movq $0xa,0x8(%rsi); addq $0x10,%rsi`) where it did 1. The same in the sib and the ics copy. The match loop and the 2 prefixed steps get 9 copies each (+72 B). Whole binary (XV, all-symbol nm compare): mid to end changes exactly 23 function sizes in each build (the 21 + `ops::forward_or_call` + `Isolate::new_with`). Jit against jitless at one commit changes 4 sizes, all cold.
- **No extra push, pop, spill, call or lost tail call (M).** In all 21 changed symbols, in both builds, the counts of push, pop, `%rsp` memory operands, calls, returns, indirect jumps, tail jumps and local branches are equal at mid and end. XV re-did 10 per-symbol diffs from the listings; each one has the fill change and the same push and `%rsp`-memory counts. The sib call copies have 2-6 more `pop` than the ics copies (an extra exit to `record_call_next`), the same `push`, and no spill. GetNamedProperty and SetNamedProperty are equal in sib and ics.
- **One jit-only codegen delta, small and cold (M):** the sib `CallUndefinedReceiver` got another register choice (+1 `movq %rdx,%rax` at entry, +2 B). It is not in the TS cycle list of either build.
- **Placement moved, and differently in the two builds (M).** Growth in front of GetNamedProperty: jit +237 B (start +224 B), jitless +295 B (start +288 B). Every hot handler of both builds has a new start address mod 64 (jit 21 of 23 hot rows, jitless 22 of 23). The dispatch tables moved too (start mod 64: jit 48 to 16, jitless 32 to 0); est. no effect, because the entries are 8 B and 8-aligned.
- **The box offset (est.).** At `m9-p5` the box TD run saved handler alignment counts. Only an offset of +32 B (mod 64) from the cross link gives the box counts for both builds (jit 119 / 235 / 469 on 64 / 32 / 16 B; jitless 126 / 234 / 469). The 16 B count is the same at every 16 B offset, and the 32 B count is the same at +0 and +32, so the fit uses 2 numbers (the 64 B count of each build) to pick 1 of 4 offsets. Both pick +32. This is good evidence for "the same handler sizes and order", and moderate evidence for the offset.
- **The sign match is 1 bit of evidence.** Both GetNamedProperty copies move by 32 mod 64 (224 and 288 B) and start 32 B apart, so at a base of 0 or 32 the two builds always move in opposite ways, and the base offset alone decides which build loses:

```
 link base offset (mod 64)   GNP jit (sib)        GNP jitless (ics)    result
 +0  (the cross link)        32 -> 0  (-1 line)   0 -> 32  (+1 line)   jit better, jitless worse
 +32 (fitted box)            0 -> 32  (+1 line)   32 -> 0  (-1 line)   jit worse,  jitless better  <- measured sign
 +16 / +48                   16 <-> 48 (0)        48 <-> 16 (0)        no GNP line change
```

- With the +32 B offset, the 20 hottest handlers weighted by cycle share: jit 8.668 to 9.111 lines of 64 B (+5.1%), jitless 9.107 to 8.814 (-3.2%) (est.).
- `.text` follows 15.8 MB of `.rodata` and `.eh_frame`, so the base offset can change with the build path or the host (I). This is a possible cause of the GitHub 9V74 sign flip; it is not checked, because that leg's JSON has no ep9 binary sha.
- **Limits:** static only; 64 B lines and alignment are not time; the box offset is fitted, not read from box addresses; the cycle weights are `m9-p5` (GetNamedProperty changed from 476 to 627 B between `m9-p5` and mid).

### 9.2 X2: dynamic counts on TS (XV verdict: CONFIRMED)

**Method.** A temporary counter (a patch that was never committed) counts one entry per handler at every dispatch, with the inner opcode of a `Wide` / `ExtraWide`; the counts per site are joined with the static `--print-bytecode` dump. Dead test: an Ldar is dead if no path reaches an accumulator read before an accumulator write, by the `acc:` table of `crates/ep9_bytecode/src/opcodes.rs` (backward liveness to a fixpoint; exception edges are not followed, because a handler starts with the exception in the accumulator, `crates/ep9_vm/src/error.rs:309-311`; `JumpLoop` writes the accumulator, `crates/ep9_bytecode/src/opcodes.rs:437-438`). XV re-derived every total with its own parser and a forward dead test: the same numbers to the unit. In 20 sampled dead `S>` sites (10 by count, 10 random), the first accumulator user is a write in all 20 (GetNamedProperty 15, Ldar 4, LdaZero 1). The jit counter run and a second jitless run (another hash seed) differ in 0 count lines.

| Lever | Kill line | Measured | Verdict |
|---|---|---|---|
| (a) elide the dead statement-position `S> Ldar` (kept by H24 R4 / rule O9) | fewer than 100 M dead: drop (the critic's kill test, 100 M line) | **151,968,271** dead `S> Ldar` runs; 127,752,522 of them are `S> Ldar <this>` | **KEEP** (1.52 times the line) |
| (b) a front-end feedback-slot order (hot slots get the low indices) | fewer than 10 M prefixed ops left | 41,058,840 now; with the best possible order (an oracle from these exact counts) **24,468,058 left** | **DROP** as a stand-alone lever; a slot order cannot remove jump offsets |

**Totals (M).** 1,657,514,120 dynamic bytecodes. Star 373,415,409 (22.53%), GetNamedProperty 347,721,884 (20.98%), Ldar 261,755,534 (15.79%): 59.3% of all dispatches.

**Ldar classes (M):**

| Class | Dynamic | % of Ldar | Static |
|---|---:|---:|---:|
| all Ldar | 261,755,534 | 100% | 16,001 |
| dead | 198,699,325 | 75.91% | 11,424 |
| dead, `S>` | 151,968,271 | 58.06% | 8,032 |
| dead, `S> Ldar <this>` | 127,752,522 | 48.81% | 4,789 |
| dead, no mark | 46,731,054 | 17.85% | 3,392 |
| live | 63,056,209 | 24.09% | 4,577 |

- The next instruction of a dead `S> Ldar <this>` is `GetNamedProperty <this>` in 108,263,751 runs (84.7%).
- Cross-check with the DEAD-LDAR probe, which removed the no-mark class: 46,731,054 x 8 x86 instructions = 0.684% of 54.69 G, against the measured -0.671% / -0.676% (DL M10-P0-DEADLDAR). The class it kept (`S>`) is 3.3 times larger.

**Prefixed ops by cause (M):** jump offset 20,969,150 (51.07%); feedback slot 17,857,974 (43.49%); slot + jump (`JumpLoop`) 603,385 (1.47%); immediate 1,628,190 (3.97%); constant 141; register 0. Total 41,058,840 = 2.48% of all dispatches. A slot is involved in 18,461,431 = 44.96%; the static 85.9% of WIDE-READ is a static share.

**Savings (est.):**

```
 lever                          ops removed     x64 instructions               share of 54.69 G   cycles (est.)    of the bucket gap vs node
 (a) dead S> Ldar               152.0 M         152.0 M x 8 = 1.216 G          2.22%              0.57 G           78% of Ldar +0.73 G
 (a') all dead Ldar             198.7 M         1.590 G                        2.91%              0.75 G           ~100% of +0.73 G
 (b) slot order (oracle)        16.6 M Wide     16.6 M x 20-30 = 0.33-0.50 G   0.61-0.91%         0.13-0.20 G      22-34% of Wide +0.59 G
```

- The x86 Ldar handler is 8 instructions (read in the X1 listing; the x86 cachegrind of `m8-p5`, a month-9 read, gives 8.0 Ir per run). The cycles per Ldar (3.76) come from the `m9-p5` bucket for the same 261.76 M runs; the bytecode is byte-identical.
- 54.69 G is the jit Ir; against the jitless 54.08 G the dead `S>` share is 2.25%.
- The (b) gain rests on an unmeasured overhead of 20-30 instructions per prefixed op; the kill stands on the measured 24.47 M > 10 M alone. Oracle slot order + branch islands on the 192 mostly-not-taken jump sites: about 13.4 M left (I), still above 10 M.
- **The counts carry to x86 (M):** the front end and the bytecode crate have one `target_arch` use, in a test helper (`crates/ep9_frontend/src/worker.rs:385-388`), and no Cargo features; the dumps of the jit, the jitless and the older builds have the same sha256 (288c4ded...); the a64 counts equal the x86-64 cachegrind dispatch counts of `m8-p5` to the unit (Star, GetNamedProperty, Ldar, Mov, ExtraWide; Wide differs only by the 242,116 int32 Smi-op runs inside `Wide`).
- **Star (M, no verdict):** 221,865,564 Stars (59.4%) follow a scale-1 GetNamedProperty; at least 330,993,484 (88.6%) follow an opcode of V8's Star lookahead set. In ep9 a lookahead saves about 0 instructions (STAR-LA2: 0 x64 Ir) and changes branch behavior (§3.2).

---

## 10. Refuted and unproven claims

### 10.1 Refuted (in the form first stated)

| Claim | By | Why |
|---|---|---|
| "JIT code is 0.9% of TS cycles" | A F7, B1, C10 | the bucket is unsymbolized samples; node --jitless has 0.7% there (VE F7, B1) |
| "PGO got +1.2-1.4% on TS" | A §9 | a sign error: jit TS 1.4% slower with PGO (DL M10-P0-WPRE-2) |
| "rank levers by x64 Ir; cut Ir and x86 time follows" | A F1, §8 | LEVER-12 -2.1% Ir gave 0 time; the per-lever factor is 0 to 1 (VE F1) |
| "LEVER-13 changed per-table codegen: extra pushes or spills in the sib call copies" | B3 (as a hypothesis) | X1: the handler code is equal in both builds; no push, pop or spill change (XV CONFIRMED) |
| "85.9% of the prefixed ops are slot overflows" (read as dynamic) | WIDE-READ, the report draft | X2: dynamic 45.0% slot, 51.1% jump offset |
| "a front-end slot order removes most Wide ops" | the critic (hypothesis) | X2: 24,468,058 left at best (> 10 M) |
| "11.5 box-h were still free" | C §3.3, §4 | the 15.9 already held the month-end lines; slack about 1.2-1.5 (VL §3) |
| "the end boxes were already running at the product call" | C §4 | the call at 07:35, the boxes started at 09:33:56 (VL C5) |
| "jit is 1.6% better than at the month-8 end"; "month-10 jit +1.0%" | C §0 | -0.7% and +1.7% (VL C1) |
| "half of the jit fall is a box move" | D I-2 | node/QuickJS is flat; the box move cancels in the paired ratio (VL D4) |
| "the tag-CI wait is 30% idle" | E §0 | it overlaps the next W-pre (VL E3) |
| "LEVER-13 is the whole jit fall; a revert gives back about 4%" | C §1.3, E §0 | the shift flips sign on GitHub 9V74 (VL C2, E5); X1: LEVER-13 changed the same 21 symbols in the same way in both builds, so a revert acts on the jit-vs-jitless gap only through placement |
| "the interpreter runs > 99% of TS" | C §5 | the handlers are 74.1%; the rest is runtime code (VL C10) |
| "8 + 1 failed Mac tries" | D §0 | 7 + 1 of 8 failures (VL D3) |
| "X86-ALIGN was stopped by G8" | E §1.4 | by the X86T time rule (VL E7) |
| month-8 tokens 37.3 M; 3.1 M per 1% time | C §3.3 | 46.83 M; 3.8 M (VL C7) |
| "the gates refused real gains" (STAR-LA2, X86-ALIGN) | C §0 | one box each; not shown real (VL C4) |
| "about 75% of TS cycles run in ops that record nothing, each in its own copy" | B §4 R3 | 37 handlers, Ldar and Star among them, are already shared; about 31% is shareable (VE B9) |
| the x64 Ldar decode is "3 instructions" | A §0 | 4 (VE F2) |
| "most of the cycle gap is backend stalls" | VE B5 | about half backend, about 40% frontend (the critic, from the TD slot files) |
| "ToBoolean and the native call path are part of the gap" | the report lever list | ep9 is already cheaper than node in both buckets (§2.4) |
| "2 load rows ≤ 3.0 today" | D §2 | 1 row (§4.2) |
| E's X2 bar (cycles width ≤ 0.5 times wall) as a fix | E §3 | the existing data gives 0.59-0.79 (VL §5) |

### 10.2 Unproven (keep as hypotheses)

| Claim | By | What would settle it |
|---|---|---|
| the jit fall is a layout effect | A F7, B3, X1 | the layout-pin row (§7 row 1); the codegen arm is already killed |
| the box link sits +32 B (mod 64) from the cross link | X1 | `nm -S` of the hot handlers of a box binary (about 1 min in any box run) |
| a ±5% layout draw per build | B6 | a 4-way link spread on one box (VL §8) |
| the month-10 jitless gain is layout luck | B8 | counters on the mid and end binaries |
| the x64 frames and decode cost x86 time (S1 share 5-10%) | A F2 | the REG-SLOT K2; per-symbol IPC (VE §5) |
| the 4-load feedback chain costs about 17% per load | A F6 | per-symbol IPC against V8, Star included (VE §5) |
| "item 3 passes in month 12" (the C9 projection) | C §3.5, §8 | the inputs are not measured (VL C9) |
| the +9..+18% bundle (A §5) and B's R1 +4..+6% | A, B | each part needs its own time read; GUESS ranges |
| "PGO failed because of the wrong training set" | A §9 | PGO with a compiler-like non-gate set (A E7) |
| "a month per VOID block" | E | count the GW12b and GW12c outcomes |
| "4 of 6 tag CIs ran on docs-only deltas over a green CI" | E §0 | read the P0-P2 merge-head CI results (VL §4) |
| the elision of dead `S> Ldar` gives x86 time | X2 | its own time A/B on the pinned layout (§7 row 4) |

---

## 11. Open questions

| # | Question | Experiment | Kill / confirm line | Cost |
|---|---|---|---|---|
| Q1 | Why do the A/B boxes run 8.0-8.8 s (IPC 1.85-1.95) when the TD box ran 6.2 s (IPC 2.46) at near-equal instructions? This confounds every one-box verdict (VL C8) | the Path B harness (4 engines) against the A/B harness (6 binaries) on one box, the same binaries, with a box-facts diff | find the cause | 0.5 box-h |
| Q2 | Do the §2.4 buckets hold on `m10-p5` code? | a TD re-run at 591064fc (perf stat + record, 4 engines) | Ldar, Wide and named load still ≥ +0.5 G each: confirm | 1 box, about 0.4 box-h |
| Q3 | Where exactly are the +6.4 G backend stall slots? (VE B5) | perf record with per-symbol IPC: ep9 GetNamedProperty + Star against V8 GetNamedProperty | ep9 cycles per load (Star included) ≤ V8's: the feedback chain is not a cause | 1 box, 0.5 box-h |
| Q4 | Does the LEVER-13 shift keep its sign per host on a repeat? (VL §2) | the LEVER-13 A/B on CF + one GitHub 9V74 Linux leg | same sign on both: not a host-specific layout draw | 0.5 box-h + 1 runner-h |
| Q5 | What is the build config of the linux-x64 node 24.22 binary? | read `process.config` of that binary inside any box run | differs from the darwin-arm64 copy | about 1 min in a box |
| Q6 | Does BOLT remove the layout draw and a part of the frontend gap? | perf record + llvm-bolt on the `m10-p5` jit binary, TS 31 rounds | < 1% gain and no lower spread: kill | about 1 box-h |
| Q7 | How does item 3 read on the named gate host model (EPYC 7763)? | §7 row 7 | jitless < 0.95 there: a CF pass is not safe | 1-4 runner-h |
| Q8 | Does Rust (the pinned toolchain) offer a handler ABI with no callee-saved registers (`preserve_none`)? (A E5) | read the toolchain features (C §6 says unstable) | - | 0 |
| Q9 | How much of V8's a64 instruction count is arm64-only overhead (BTI, PAC)? It changes the §2.1 split by a few points (A E6) | a count on a V8 build without them | - | a local V8 build |
| Q10 | The month-8 agent-run count: 268 or 275? | recount the usage-log rows, outcome text included | - | 5 min |

## Raw numbers

[artifacts/m2-stuck/](artifacts/m2-stuck/README.md) holds `findings.json`: the ranked causes and their verdicts, the lever ledger rows, the box-neutral series, the per-host table, the gate cells, the process costs, the decisions D1-D7 and the X1 and X2 numbers, each with its source.
