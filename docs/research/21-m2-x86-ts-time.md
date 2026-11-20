# x86 TS time for the M2 gate: the need per cell, the Ir-to-time factor, the real counters, the V8 forms and the month-10 levers

## Run context

| Item | Value |
|---|---|
| Date | 2026-11-19 (section 9 addendum: 2026-11-21) |
| Round | The month-10 plan research (speed part), its design audit, and plan v15 §0 and §10. The plan decisions are rows M10-PLAN-D1 to -D53 in [DL §20.1](../decisions-log.md) (plan id V15-Dn = row M10-PLAN-Dn) |
| ep9 tree | tag `m9-p5`, 86e39c65 (`86e39c6530371bac073162c70b9461f06328831c`). Every ep9 code and ADR line cite in this report is at this commit |
| ep9 docs | DL and MR9 line cites are at the month-end docs head 15f2ba11 (`m2m9/month-end`). §20 is appended after these lines, so they do not move |
| x86 readings | (1) The month-9 Path B: 2 Cloudflare boxes, AMD EPYC 9684X (Zen 4; family 25, model 17), 4 vCPU, the `m9-p5` binaries, 31 paired rounds per build, 2026-11-18 (DL M9-END-D5). (2) The X86-JITLESS-READ box: 1 Cloudflare box of the same type with perf counters, `m8-p5` code (06486612), a function-alignment A/B with 30 measured rounds after a warm-up round (DL M9-P0-D1). (3) The Windows Path B: GitHub windows-x64 runners, AMD EPYC 7763 (Zen 3) (DL M9-END-D6) |
| Mac reading | GW11b on the Mac a64 gate machine, `m9-p5` (DL M9-END-D9; MR9:107-115) |
| Ir readings | cachegrind Ir from the GitHub bench mode (rule 13): run 43819528550 (base fd9c12ce = the `m8-p5` code) and run 44303131944 (base 86e39c65), the `octane-typescript` base rows, x64 and a64, jit and jitless |
| Upstream | `v8/v8@40f4b69c3` (`40f4b69c3bc25f2a9c1c4137e27c545b6b1183d0`, local clone of 2026-11-02); `bellard/quickjs@712e8c0c` (`712e8c0ca26949477c38f91dbaf955cc48e2f261`, local clone of 2026-11-02) |
| Method | Read only, 0 compile, 0 boxes. One listing was printed with the `m9-p5` release binary (`ep9 --print-bytecode`, §5.1). Derived numbers show their inputs. "est." marks an estimate |

Short forms:
- DL = [docs/decisions-log.md](../decisions-log.md). MR9 = [docs/measurements/2026-11-19-m2-month9.md](../measurements/2026-11-19-m2-month9.md). `Hnn` = `docs/adr/Hnn-*.md`. `09:66` = `docs/plan/09-milestones.md` line 66. To read an ep9 cite later, use `git show m9-p5:<path>`.
- PB = the month-9 Path B compare (2 boxes). MR9:136-318 prints it (TS: MR9:155, :159; jitless over jit: MR9:302).
- XJR = the X86-JITLESS-READ box summary (`m8-p5`; DL M9-P0-D1, DL:5313). Its section C is the alignment A/B with perf counters.
- Upstream cites read `owner/repo@rev path:line`.
- A "cell" is one build (jit or jitless) on one host (one box or one runner). "×1.11" means "the ratio must grow by 11%".
- Ratio = node --jitless time / ep9 time, paired per round (above 1.0 = ep9 is faster). Item 3 on Linux x64 needs the interval low end ≥ 1.0 (09:104-113).

---

## 0. Verdict

```
 1 char = 1% of x86 TS time (ratio to node --jitless)

 need (low end to 1.0)   linux-x64 jit      |███████████▎        ×1.112-1.114
                         linux-x64 jitless  |██████████████▌     ×1.144-1.145
                         win-x64 jit        |█████████████▉      ×1.138-1.145
                         win-x64 jitless    |███████████▍        ×1.109-1.119

 month-10 levers, middle, with the measured Ir-to-time factors (§6.3)
                         jit                |████▋               ×1.046
                         jitless            |██████              ×1.060

 rest for month 11       jit ×1.064, jitless ×1.080  -> only PGO (est. +3..+10%) or
                                                        REG-SLOT-ENCODE (Ir -3.4..-5.1%, upper bound)
```

1. **Every x86 TS cell is below 1.0, and needs ×1.11 to ×1.15.** Linux x64 jit 0.902 / 0.914, jitless 0.888 / 0.893 on the 2 boxes (MR9:155, :159). Windows x64 jit 0.881 / 0.880, jitless 0.898 / 0.903 (MR9:341, :345). §1.
2. **Ir is a weak x86 time signal.** Month 9 cut x64 TS Ir by 6.24% (jit) and 6.38% (jitless). The x86 TS ratio rose only +2.89% and +2.00%. So 1% of x64 TS Ir bought 0.46% (jit) and 0.31% (jitless) of x86 time. On the Mac a64 the factor was 0.69 and 0.49. §2.
3. **The x86 penalty did not move.** Mac ratio / x86 ratio = 1.157 / 1.192 in month 8 and 1.156 / 1.194 in month 9 (jit / jitless). The x64-only Ir cuts of month 9 bought no x86-only time that the readings can see. §2.1.
4. **Real branch misses are 12 times fewer than the cachegrind model says.** On TS (jit, Zen 4): 103.1 M real misses, 1,258 M simulated (Bcm + Bim). IPC is 2.19. So a Bim row is a note, never a lever reason. §3.
5. **The only measured x86-specific time lever is layout.** Every `ep9h__` handler at 64 B: jitless TS wall 0.975 [0.958-0.984], instructions 1.000 [0.998-1.001]. That is a front-end effect, not prediction. §3.
6. **V8 jitless does not pay two ep9 costs.** (a) The lazy accumulator: V8 never emits a binding `Ldar` whose value the next bytecode overwrites. In ep9 69.87% of the static TS `Ldar` sites are such loads. (b) The Star lookahead: 37 producers run the next short `Star` inline and dispatch from a second site for branch prediction. §5.
7. **The month-10 levers do not close the need.** The plan research summed them to a middle of ×1.075 (jit) and ×1.094 (jitless). That sum used a DEAD-LDAR time row that needs 0.6-1.9 time per 1% Ir. With the measured factors (0.31-0.69) the middle is ×1.046 / ×1.060. Month 11 then needs ×1.064 / ×1.080 more. §6.
8. **What plan v15 took** (§7): month 11 is the planned M2 exit month, with a pre-registered switch (M10-PLAN-D1). Measure x86 time first (M10-PLAN-D6). A time path for rule 13 (M10-PLAN-D7). DEAD-LDAR, X86-ALIGN and STAR-LA2 in M10-P0 to M10-P2 (M10-PLAN-D8 to -D10). PGO-PROBE in the pre-step (M10-PLAN-D39). A read-only REG-SLOT-ENCODE study (M10-PLAN-D53).

---

## 1. The need per x86 TS cell

The host-cell rule passes a cell only when the low end of the interval is ≥ 1.0 on both boxes, in each build (the reader of record is `cargo xtask m2-exit --pathb`, `tools/xtask/src/m2_exit.rs:45-56`). "Median ×" = 1 / median. "Low end ×" = 1 / low end, if the interval keeps its width.

| Cell | Median [interval] | Median × | Low end × | Source |
|---|---|---|---|---|
| linux-x64 jit, box 1 | 0.902 [0.898-0.911] | 1.109 | **1.114** | MR9:155 |
| linux-x64 jit, box 2 | 0.914 [0.899-0.920] | 1.094 | **1.112** | MR9:155 |
| linux-x64 jitless, box 1 | 0.888 [0.874-0.898] | 1.126 | **1.144** | MR9:159 |
| linux-x64 jitless, box 2 | 0.893 [0.873-0.911] | 1.120 | **1.145** | MR9:159 |
| win-x64 jit, runner w3 | 0.881 [0.873-0.883] | 1.135 | **1.145** | MR9:341 |
| win-x64 jit, runner w4 | 0.880 [0.879-0.883] | 1.136 | **1.138** | MR9:341 |
| win-x64 jitless, w3 | 0.898 [0.894-0.900] | 1.114 | **1.119** | MR9:345 |
| win-x64 jitless, w4 | 0.903 [0.902-0.906] | 1.107 | **1.109** | MR9:345 |

- So: linux-x64 jit ×1.11, linux-x64 jitless ×1.14-1.15, win-x64 jit ×1.14-1.15, win-x64 jitless ×1.11-1.12 (derived).
- **The month-9 move** (the 2-box mean of the medians, PB group G): jit 0.8825 -> 0.908 (+2.9%), jitless 0.873 -> 0.8905 (+2.0%) (derived from the month-8 medians 0.884 / 0.881 and 0.872 / 0.874, DL M8-END-D4).
- **Windows by the milestone text** (09:66: Windows x64 "stays within 10% of Linux x64"). The cross-host interval low end = Windows low / Linux high: TS jit 0.873 / 0.920 = 0.949, TS jitless 0.894 / 0.911 = 0.981 (derived). Both are ≥ 0.9. But these runners are Zen 3 and the boxes are Zen 4, so the ratio also measures the CPU kind. Plan v15 reads the 09:66 form only on pairs of the same CPU model (M10-PLAN-D16).
- **Mac a64, for comparison** (GW11b, DL M9-END-D9): TS gross 2.628 s [2.571-2.969] (jit, provisional) and 2.555 s [2.529-2.797] (jitless) against the Mac line 2.8 s; the TS ratio to node --jitless is 1.050 (jit) and 1.063 (jitless) (MR9:111-112). Block A was VALID in run 2; block B was MISSING (outside load over the guard line 3.005); block C was VALID with JetStream list ratios 0.537 / 0.549 (DL:5611). The Mac TS cells are open too: the jit upper end 2.969 s is over 2.8 s (a provisional half), and the jitless upper end 2.797 s is 3 ms under it (MR9:76, :111). A quiet window can close the Mac cells; the x86 cells need code.

---

## 2. The Ir-to-time factor

### 2.1 The x86 penalty

| Quantity | Month 8 | Month 9 | Source |
|---|---|---|---|
| Mac TS ratio to node --jitless, jit / jitless | 1.021 / 1.041 | 1.050 / 1.063 | MR9:112 |
| x86 TS ratio, jit / jitless (2-box mean of the medians) | 0.8825 / 0.873 | 0.908 / 0.8905 | DL M8-END-D4; MR9:155, :159 |
| **x86 penalty = Mac ratio / x86 ratio**, jit / jitless | **1.157 / 1.192** | **1.156 / 1.194** | derived |
| x64 / a64 TS Ir, jit / jitless | 1.119 / 1.121 | 1.094 / 1.096 | derived from the bench base rows (below) |

The bench base rows of `octane-typescript` (cachegrind Ir): fd9c12ce (the `m8-p5` code, run 43819528550): x64 60.53 G (jit) / 60.00 G (jitless), a64 54.10 G / 53.55 G. 86e39c65 (`m9-p5`, run 44303131944): x64 56.75 G / 56.18 G, a64 51.87 G / 51.25 G.

- The month-9 gains were platform-neutral: the Mac ratio rose +2.8% / +2.1% and the x86 ratio +2.9% / +2.0% (derived).
- The x64/a64 Ir ratio fell about 2.2% (the x64-only levers X86-WIDE-FAST and X86-STORE-R1), but the penalty stayed. Either x64 Ir is not where the x86 time is, or the readings cannot see about 1% (2.2% × 0.3-0.5). Both say the same: the x64-only Ir path is nearly spent.
- To pass on x86 with this penalty, the Mac ratio must reach about 1.17-1.18 (jit) and 1.21-1.22 (jitless) (derived: 1.156 × 1.01-1.02; 1.194 × 1.015-1.02). So month 10 needs both platform-neutral gains and penalty cuts.

### 2.2 Time per 1% Ir (month 9)

| | TS Ir change | TS time change (ratio) | Time per 1% Ir |
|---|---|---|---|
| x86 jit | -6.24% (x64) | +2.89% | **0.46** |
| x86 jitless | -6.38% (x64) | +2.00% | **0.31** |
| Mac a64 jit | -4.13% (a64) | +2.84% | **0.69** |
| Mac a64 jitless | -4.30% (a64) | +2.11% | **0.49** |

Derived from the §2.1 rows.

- **For an x64-only Ir lever on x86, use 0.31-0.46.** Plan v15 rule 81 makes each x86 estimate name this factor (M10-PLAN-D6).
- **For a platform-neutral lever, use 0.31-0.69** (the x86 and the Mac readings together). The §6.3 sum uses this range.
- The factor depends on the lever kind. A cut on a memory or a dispatch path moves time more than a cut of ALU instructions inside a handler. The month-8 TS-STORE lever is the example where Ir moved time well (x64 TS Ir -7.36%; the month-10 plan research).
- **Limit.** The factor comes from one month and from paired ratios with box noise (§8 K1).

---

## 3. Real counters against cachegrind (Zen 4 box, `m8-p5` code)

| TS jit run (XJR section C) | Value |
|---|---|
| wall, cycles, instructions (median of 30 rounds) | 7,564.6 ms; 27.77 G; 60.82 G |
| IPC; clock | 2.19; 3.67 GHz (derived) |
| real branch misses (all kinds) | 103.1 M, MPKI 1.70 (derived) |
| cachegrind mispredicts of the same workload (Bcm + Bim) | 396.5 M + 861.9 M = 1,258 M (12.2 × the real count; derived) |
| share of the cycles in real branch misses at an est. 12 / 15 / 20 cycles each | 4.5% / 5.6% / 7.4% (derived) |

| Alignment A/B: every `ep9h__` handler at 64 B (jitless-al) against the plain jitless build | Median [interval] |
|---|---|
| TS wall, jitless-al / jitless | **0.975 [0.958-0.984]** |
| TS cycles, jitless-al / jitless | 0.974 [0.957-0.981] |
| TS instructions, jitless-al / jitless | 1.000 [0.998-1.001] |
| TS branch misses, jitless -> jitless-al | 101.7 M -> 103.9 M |
| TS wall, jitless / jit (the jitless loss) | 1.021 [1.012-1.034] |
| share of the jitless TS wall loss that the alignment removed | 131% (derived in XJR: 1.308) |
| crypto wall, jitless-al / jitless | 0.897 [0.871-0.932] |

Reading:
- The dispatch tails hold most of the simulated Bim (the `ep9h__` handlers hold 94.9% of the jit TS Bim; derived from the per-function cachegrind table of the same run). But the real miss rate is low. Zen 4 predicts the dispatch far better than the cachegrind model. So a lever that is sold by Bim (for example STAR-FUSE, or a Bim move in one dispatch tail) must be read in time on a box, not in Bim. Plan v15 makes Bim rows notes (M10-PLAN-D6).
- The alignment gain came with flat instructions and flat branch misses. So it is a front-end effect (fetch and the op cache), not prediction.
- IPC 2.19 on a 6-wide core leaves cycles that no reading explains yet. They are most likely front-end and load latency (est.). The box has perf counters, but no top-down or per-symbol cycle profile was taken. That is the first month-10 measure: X86-TOPDOWN-READ (plan v15 §2 step 9 (a)).
- At `m9-p5` the jitless build is still slower than the jit build in the same rounds on x86: TS 1.017 [1.005-1.026] (box 1) and 1.018 [0.996-1.042] (box 2) (MR9:302).
- The ADR keeps this lever closed today: "Alignment helps Zen 4 and Intel, but stable Rust can only align every function of the binary (LLVM flag). Not applied." (`docs/adr/H15-interpreter-dispatch.md:39`). Layout moves 9-22% between builds on the Zen 3 loop kernel (H15:41).

---

## 4. The TS hot paths on x86

At `m8-p5` (the per-function cachegrind table of the XJR run; groups and shares derived):

```
 x64 TS jitless, 59.96 G Ir                                 share of Ir   share of simulated Bim
 IC/shape  (GetNamedProperty 25.4, SetNamedProperty 7.0,     41.8%         GNP 18.7, SNP 3.3, GNP_cold 1.4
            GetNamedProperty_cold 4.2, SNP_cold 2.0, ...)
 dispatch/handlers (Star 5.0, scaled::step<S2> 3.6,          26.7%         Star 27.2, Ldar 18.2, Wide_cold 2.5,
            Ldar 3.5, LdaCurrentContextSlot 1.4, ...)                      scaled::step 2.3, Return 5.0
 calls (slow::call 4.9, CallProperty0/1/2 8.9, ...)           18.8%
 allocation/GC 3.5%, property maps 3.2%, builtins 3.0% (char_code_at 2.1), strings/atoms 1.5%, compiler 1.2%
```

- **Register moves.** Star 373.4 M and Ldar 261.8 M dispatches on TS (DL M9-P0-D1), 8 instructions each (XJR). node dispatches `Ldar` 62.4 M times on the same workload (4.2x fewer; a month-8 cachegrind study, the month-10 plan research).
- **Register operand decode.** Each handler maps the operand to a slot with a sign test (`R::from_operand`, `crates/ep9_vm/src/interp/dsl.rs:86-97`: a parameter sits below the H14 frame header). On x86-64 that is `movsbq; lea -0x8; test; cmovns`: 3 more instructions per register operand than one load. On Star, Ldar and Mov alone that is 3.4% of x64 TS jit Ir, and 1.7% more with the GetNamedProperty object operand (derived, upper bound). V8 puts the frame offset into the operand, so its decode is one load.
- **IC misses.** At `m8-p5` SetNamedProperty goes cold 10.46 M of 65.51 M times (16.0%), GetNamedProperty 54.69 M of 347.72 M (15.7%) (derived from the per-handler indirect-branch counts). The cause is not read. The code has a per-site miss counter (`ic::ic_count!(miss, Store, st.info(), s.0)`, `crates/ep9_vm/src/interp/ops.rs:1189`; feature `ic-stats`, `crates/ep9_vm/src/ic.rs:2128-2132`). After month 9 (bench log 44204846399 134468049051:1505-1525; x64 TS jit): `SetNamedProperty_cold` 2.09%, `shape::lookup_with` 0.67%, `set_named_miss` 0.45%, `ic::grow_add::<true>` 0.35% of Ir.
- **Prefixed operands.** `scaled::step::<S2>` 3.62%, `Wide_cold` 0.72% and `Wide` 0.53% of x64 TS jitless Ir at `m8-p5`: 4.9% for 39.2 M prefixed instructions, about 74 Ir each (`scaled::step` alone about 55) (derived). On x86-64 T0, every prefixed instruction except LdaSmi and the int32 Smi ops runs in `scaled::step`. Which operand overflows (the constant index, the feedback slot or the register) is not read.
- **Static TS shares** (the month-9 TS bytecode count, DL M9-P0-D1): Star 28.3% of the instructions, GetNamedProperty 19.8%, Ldar 12.8%. 81.3% of the GetNamedProperty sites feed a Star.

---

## 5. How V8 jitless and QuickJS avoid these costs

| Cost in ep9 T0 | V8 jitless (Ignition), the item-3 comparator | QuickJS (item 1 only) |
|---|---|---|
| A dead binding `Ldar` (69.87% of the static TS `Ldar` sites, §5.1) | **Lazy accumulator.** `BytecodeArrayBuilder::LoadAccumulatorWithRegister` calls `DoLdar` (`v8/v8@40f4b69c3 src/interpreter/bytecode-array-builder.cc:753-759`). `DoLdar` is a register transfer, not an emit (`src/interpreter/bytecode-register-optimizer.h:53-58`; `RegisterTransfer`, `src/interpreter/bytecode-register-optimizer.cc:456-483`). The accumulator is materialized only when a bytecode reads it (`.h:94-99`). Before a bytecode that writes the accumulator, the optimizer only keeps a materialized copy of the old value in another register (`.h:101-105`). After `DoLdar` the binding register is that copy, so nothing is emitted. All state is flushed at a jump, a switch, a debugger call and a generator suspend or resume (`.h:75-92`), so the elision holds inside a basic block | A stack machine: `get_loc0..3` and `put_loc0..3` (`bellard/quickjs@712e8c0c quickjs.c:19050-19057`) |
| A `Star` after a producer | **Star lookahead.** `Bytecodes::IsStarLookahead` lists 37 producers at the single operand scale (LdaX, GetNamedProperty, GetKeyedProperty, the context-slot loads, Add, Sub, Mul, Inc, Dec, TypeOf, the Call and Construct forms, the object and array literals, GetTemplateObject; counted) plus `kDebugBreak0` (`v8/v8@40f4b69c3 src/interpreter/bytecodes.cc:261-307`). `StarDispatchLookahead` runs the short Star inline and dispatches from a second site: "we can get better branch prediction by duplicating it" (`src/interpreter/interpreter-assembler.cc:1344-1372`, the comment at :1365-1368; the call site :1405-1414) | No Star |
| Register operand decode (3 instructions, §4) | The operand is a frame slot index: one load | Locals are C locals in one function |
| Handler register state | Pinned registers: accumulator `rax`, bytecode offset `r9`, bytecode array `r12`, dispatch table `r15` (`v8/v8@40f4b69c3 src/codegen/x64/register-x64.h:363-366`). Bytecode handlers have no callee-saved registers (`src/compiler/linkage.cc:767-768`) | One function `JS_CallInternal` with a computed-goto table (`bellard/quickjs@712e8c0c quickjs.c:18218-18241`) |

ep9 T0 today: a handler is `fn name(cx, pc, fp, acc)` with the SysV ABI (`extern "sysv64"` in release, `sysv64-unwind` with unwinding) on every x86-64 OS (`crates/ep9_vm/src/interp/dsl.rs:253-280`), and it dispatches with `jmp *(%rdi,%rax,8)` (XJR disassembly of the Star handler).

### 5.1 A dead binding load, seen in the `m9-p5` listing

REGOPT rule O2 emits every load from a binding register: "Every load from a binding register (`Ldar` or `Mov` from a parameter or a named local) is emitted", with the reason H24 R4 "no binding folds" (`crates/ep9_frontend/src/bytecodegen/regopt.rs:20`; `docs/adr/H24-debug-and-profiling-hooks.md:31`).

Source: `function f(a) { let s = 0; for (let i = 0; i < a.length; i++) { s = s + a[i]; } return s; }`. The loop body of `f`, printed by `ep9 --print-bytecode` with the `m9-p5` release binary (the jitless preset; the jit preset prints the same lines):

```
      6    Ldar r1                                      <- dead: GetNamedProperty writes acc, reads a0, not acc
      8 E> GetNamedProperty a0, k0, s0  ; "length"
     12 E> TestLessThan r1, s1                          <- reads r1 by its register operand
     15    JumpIfFalse @39
     17 S> Ldar r0                                      <- dead: the next Ldar overwrites acc
     19    Ldar r1                                      <- live: GetKeyedProperty reads the key in acc
     21 E> GetKeyedProperty a0, s2
     24 E> Add r0, s3
     27    Star r0
     29    Ldar r1                                      <- live: Inc reads acc
     31 E> Inc s4
     33    Star r1
     35    JumpLoop @6, [0], s5
```

- 2 of the 4 `Ldar` per loop turn are dead. The V8 register optimizer would drop both (§5 row 1). Offset 6 is the loop head (`JumpLoop @6`), but the label is before the `Ldar`, so no label lies between the dead load and the instruction that overwrites the accumulator.
- On TS, 11,180 of 16,001 static `Ldar` sites (69.87%) are followed by an instruction that writes the accumulator and does not read it (the month-9 TS bytecode count; DL M9-P0-D1 has the totals). The next instruction is GetNamedProperty in 6,194 of them. This is a static count. The dynamic share is not measured.
- If 50-70% of the 261.8 M dynamic `Ldar` dispatches are dead, DEAD-LDAR removes 130-183 M dispatches and 1.8-2.6% of x64 TS Ir (derived), on both arches.
- The register read of the binding stays: `TestLessThan r1` and `GetNamedProperty a0` read the binding by their register operand. The ep9 guard that DEAD-LDAR must change is the REGOPT guard "a binding load is gone (H24 R4)" (`crates/ep9_frontend/src/bytecodegen/tests.rs:826-837`).

### 5.2 STAR-FUSE: a time win that the Ir gate refused

- STAR-FUSE (month 9) put a Star lookahead at the success exit of 6 producers. Mac a64 guide: TS Ir +0.454% and +0.456%, wall -2.1%. Variants with more producers gave -2.8% (11 producers) and -3.1% (the head set) (the role acceptance record; the month-10 plan research).
- The bench floor (+0.5% Ir per row) failed: linux-x64 t0-prop +1.019% (jit) and +1.020% (jitless), octane-typescript +0.568% and +0.574%. It was reverted (30d41178) because "the floor is a regression gate, and x86 wall time is not measured" (DL M9-P0-D4, DL:5316).
- The arm64 debug native-stack test failed with the fused chain (`interp/dsl.rs:308`; DL:5316). A later Star lever must pass it.
- So the lever kind is right (V8 does it), but the gate could not see the gain. Plan v15 adds a rule-13 time path for it (M10-PLAN-D7).

---

## 6. The lever list and its sum

### 6.1 The levers (est. x86 TS time unless named)

The gain column is the measured-factor row of §6.3, unless the row names another source.

| # | Lever | What | Gain (est.) | Evidence | G8 kind (plan v15) |
|---|---|---|---|---|---|
| W1 | X86-TOPDOWN-READ (measure) | 1 box ≤ 1.5 box-h: `perf stat` top-down metrics if the microVM passes them; `perf record -e cycles` per symbol for ep9 jit, ep9 jitless and node --jitless on TS; the jit build with 64 B alignment (the cell XJR did not read) | steering | §3: no per-function time reading on x86 exists | 0 |
| W2 | DEAD-LDAR | REGOPT drops a binding `Ldar` when the next instruction writes the accumulator without a read and no label lies between (the V8 lazy-accumulator form) | **+0.6..+1.8%** (x64 TS Ir -1.8..-2.6% × 0.31-0.69); both arches. The plan research first wrote +1.5..+3.5%; that needs 0.6-1.9 time per 1% Ir, more than any measured factor | §5.1 | a rewrite of the Frozen H24 R4 check; G8 +1 only if the probe meets its kill line (x64 TS Ir ≤ -1.0%) (M10-PLAN-D8) |
| W3 | STAR-LA2 | the STAR-FUSE body form with more producers and the V8 duplicated dispatch site, under the time path | x86 +0..+3% (the x86 case came from Bim, so the low end is 0); Mac -2.1..-3.1% wall measured | §5.2 | 0 (body form) (M10-PLAN-D10) |
| W4 | X86-ALIGN | 64 B alignment of the `ep9h__` handlers on x86-64 (a per-handler attribute if the toolchain has one; else an x86-64 target flag) | jitless +1.7..+2.5% (the m9-p5 jitless/jit loss 1.7-1.8% up to the measured m8-p5 gain); jit 0..+1.5% (not measured) | §3 | +1: a rewrite of H15:39 (M10-PLAN-D9) |
| W5 | NAMED-MISS | read the per-site miss histogram of SetNamedProperty and GetNamedProperty on TS; cut the largest miss class | +0..+1.5%; both arches | §4 IC misses | an IC list append is a fill; a change of the H16 growth rule is not planned |
| W6 | WIDE-READ -> WIDE-CUT | count the prefixed instructions by opcode and by the operand that overflows; then a front-end order or an x86 per-scale table | +0..+0.7% (x86 only; an x64-only Ir lever, × 0.31-0.46) | §4 prefixed operands | per-scale table = H15 dispatch core rewrite (stop and log) |
| W7 | WIN64-JIT | the Windows jit loss: on Windows jit is under jitless (jitless / jit 1.019 on w3, 1.026 on w4), on Linux jit is over jitless (jit / jitless 1.016 on box 1, 1.024 on box 2) (derived): a Windows-only jit loss of about 4% | win-x64 jit +2..+4% | MR9:341, :345; MR9:155, :159 | 0; only if a win-x64 cell reads low at the mid Path B |
| W8 | PGO-PROBE | 1 box A/B: a PGO build trained on a set with **no gate workload** against the plain build | +3..+10% (no ep9 data) | PGO changes block layout and inlining, the cost class of §3 | a product and build-rule call; probe only (M10-PLAN-D39) |
| W9 | REG-SLOT-ENCODE | encode a register operand as its frame slot offset, so the decode is one load | TS Ir -3.4..-5.1% (upper bound); both arches | §4 register operand decode | bytecode format and H14 frame mapping: G8 ≥ +1; a read-only study in month 10 (M10-PLAN-D53) |
| W10 | PREALLOC + BESTFIT | the M10-P0 base merges | +0.1..+0.2% (about -0.4% TS Ir × 0.31-0.46) | DL M9-END-D12 (DL:5614): TS x64 -0.217% / -0.219% (PREALLOC), -0.178% / -0.179% (BESTFIT) | 0 |

Not item-3 work: GLOBAL-CELL, KEYED-NAVIER-DOUBLE, H15-ABI-PRESERVE-NONE (needs an unstable Rust ABI), LDAR-ELIDE (W2 supersedes it).

### 6.2 The plan research sum (multiplicative)

| Build | Low | High | Middle | Need (§1) |
|---|---|---|---|---|
| linux-x64 jit | ×1.036 | ×1.115 | ×1.075 | ×1.112-1.114 |
| linux-x64 jitless | ×1.062 | ×1.126 | ×1.094 | ×1.144-1.145 |
| win-x64 jit (+ W7 ×1.02-1.04) | ×1.057 | ×1.160 | ×1.11 | ×1.138-1.145 |

Factors used. Low: W2 1.015, W3 1.01, W4 jit 1.00 / jitless 1.025, W5 1.005, W6 1.005, W10 1.001. High: W2 1.035, W3 1.03, W4 1.015 / 1.025, W5 1.015, W6 1.015, W10 1.002. (A recompute from these rounded factors gives highs ×1.117, ×1.128 and ×1.162; the table keeps the research values. The difference is rounding.)

Even this sum has the jitless high end (×1.126) under the need (×1.144). The research put the chance that all 4 linux-x64 cells pass in month 10 at est. ≤ 20%.

### 6.3 The measured-factor sum (plan v15 §0)

The design audit replaced the research time rows that the readings do not support:

| Lever | Research row | Measured-factor row | Why |
|---|---|---|---|
| W2 DEAD-LDAR | 1.015-1.035 | **1.006-1.018** | x64 TS Ir -1.8..-2.6% × 0.31-0.69 |
| W3 STAR-LA2 | 1.01-1.03 | **1.00-1.03** | the x86 case came from Bim; Bim rows are notes (§3) |
| W4 X86-ALIGN jit | 1.00-1.015 | 1.00-1.015 | not measured |
| W4 X86-ALIGN jitless | 1.025 | **1.017-1.025** | the jitless loss is 1.7-1.8% at m9-p5 (MR9:302), 2.5% measured at m8-p5 |
| W5 NAMED-MISS | 1.005-1.015 | **1.00-1.015** | the cause is not read |
| W6 WIDE-CUT | 1.005-1.015 | **1.00-1.007** | an x64-only Ir lever |
| Queue levers Q4-Q8 | - | 1.00 | no TS effect |

| Build | Low | High | Middle | Need | Rest for month 11 (need / middle) |
|---|---|---|---|---|---|
| jit | ×1.006 | ×1.088 | **×1.046** | ×1.112-1.114 | **×1.064** |
| jitless | ×1.023 | ×1.099 | **×1.060** | ×1.144-1.145 | **×1.080** |

Derived: low and high are the products of the rows; the middle is √(low × high); the rest is the middle of the need over the middle. Plan v15 §0 rounds these to "about ×1.05 (jit) / ×1.06 (jitless)" and "about ×1.06 / ×1.08 more" for month 11.

- No lever of the plan v15 queue (Q1-Q8) gives ×1.06-1.08.
- Only two named levers can: **PGO** (est. +3..+10%, no ep9 data) and **REG-SLOT-ENCODE** (TS Ir -3.4..-5.1% upper bound, G8 ≥ +1, a bytecode format change).
- So PGO-PROBE runs in the pre-step (M10-PLAN-D39), and REG-SLOT-ENCODE gets a read-only study in M10-P1 (M10-PLAN-D53).

---

## 7. What plan v15 took

| Decision | DL row | What |
|---|---|---|
| The exit month | M10-PLAN-D1 | Month 11 is the planned M2 exit month. A pre-registered switch at the M10-P3 W-pre makes month 10 the exit month if the mid Path B reads every linux-x64 TS cell with both low ends ≥ 1.0 in both builds, and every win-x64 09:66 cell AGREE above |
| Measure first | M10-PLAN-D6 | X86-TOPDOWN-READ (1 box ≤ 1.5 box-h), NAMED-MISS-READ and WIDE-READ before the levers; x86 estimates by time; an x64-only Ir lever names the factor 0.31-0.46; Bim rows are notes |
| The rule-13 time path | M10-PLAN-D7 | A dispatch or layout lever may raise Ir by ≤ +1.0% per row if one x86 box A/B (31 paired rounds, both builds) puts the TS interval below 1.0 and no row above 1.0, and the Mac guide shows no TS loss |
| DEAD-LDAR | M10-PLAN-D8 | The first code lever (M10-P0): kind row first; probe kill line x64 TS Ir ≤ -1.0%; G8 +1 only if the probe meets it |
| X86-ALIGN | M10-PLAN-D9 | M10-P1; G8 +1 planned (H15:39 rewrite); a per-handler form first |
| STAR-LA2 | M10-PLAN-D10 | M10-P2 under the time path, after the cause of the arm64 debug stack failure; G8 0 |
| The lever queue | M10-PLAN-D11 | LEVER-12 to LEVER-15 from the queue Q1-Q8 (plan v15 §10) |
| Windows reading | M10-PLAN-D16 | the 09:66 form only on same-CPU-model pairs; Linux x64 keeps the absolute thresholds |
| PGO-PROBE | M10-PLAN-D39 | pre-step, after X86-TOPDOWN-READ; 1 box ≤ 1.0 box-h; a training set with no gate workload; the take-or-not row at the M10-P1 W-pre |
| This report | M10-PLAN-D52 | R21 in `docs/research`, and an addendum with the month-10 reads before plan v16 |
| The rest | M10-PLAN-D53 | REG-SLOT-ENCODE-STUDY (read only, M10-P1); a function with ≥ 3% of x86 TS cycles and no lever gets a queue row; if the mid Path B shows less than ×1.04 on any x86 TS cell, plan v16 pre-registers month 12 as the fallback exit month |

---

## 8. Limits and open risks

| # | Risk | Effect | What the plan does |
|---|---|---|---|
| K1 | The Ir-to-time factor comes from one month (2 Path B readings and 2 Mac windows) | the §6 sums are wrong in either direction | X86-TOPDOWN-READ gives time per function; the mid Path B decides the switch |
| K2 | Layout gains fade with later changes (H15:41: 9-22% between builds on the Zen 3 loop kernel) | X86-ALIGN can lose its gain, or another change can move x86 time with no code meaning | read X86-ALIGN again on the exit candidate in the Path B, not only at its merge |
| K3 | The gate host, when the owner adds one, is Zen 3 (EPYC 7763). H15:39: alignment did not fix the Zen 3 spike result | a cell that passes on the Zen 4 boxes can fail there | the Windows runners are EPYC 7763 and read TS jit 2.3-3.7% under linux-x64 and TS jitless 0.6-1.7% over it, per cell (derived from §1): a weak sign that Zen 3 is not much worse on TS |
| K4 | The dynamic dead share of `Ldar` is not measured (§5.1 is static) | W2 can give < 1% | the probe kill line (M10-PLAN-D8) |
| K5 | The top-down events may not pass the Cloudflare microVM | W1 reads cycles per symbol only | the plan accepts that form |
| K6 | Windows has no box and no cachegrind | W7 can only be read on GitHub runners | W7 is a queue row with a condition, not a planned lever |
| K7 | A PGO profile can learn the gate workloads | a gamed gate | the training set holds no gate workload (M10-PLAN-D39) |

---

## 9. Addendum: the month-10 reads (2026-11-21)

This section adds the month-10 x86 TS reads (M10-PLAN-D52; plan v15 section 13.4 step 7) on the trees from `m9-p5` 86e39c65 to the `m10-p4` head 12203b0c, and the month-end Path B on the `m10-p5` tag is in the month-10 report, not here.

Short forms of this section:
- Every number of this section is in `month10.json` (see Raw numbers), with its source: a DL row id, a GitHub run id, or "derived" with its inputs. The DL rows are in DL §20. DL M10-P0-WPRE-1 (X86-TOPDOWN-READ) and DL M10-P0-WPRE-2 (PGO-PROBE) are in DL §20.7.
- Trees: 2856a698 = the `m10-p2` tag code, the tree of the mid Path B. f09c2daa = the `m10-p3` code (LEVER-13 merged). 29ba45f6 (the `m10-p3` tag) and 12203b0c add DL text only.
- B/A = the wall time of the lever build over the base build, paired per round (below 1.0 = the lever build is faster). PGO-PROBE reads t(plain) / t(PGO) (above 1.0 = PGO is faster).
- "Box" = one Cloudflare box, AMD EPYC 9684X (Zen 4), the same type as in §1, unless a row names another host.

### 9.1 Verdict

```
 1 char = 1% of x86 TS time (ratio to node --jitless)

 R21 middle (§6.3)          jit              |████▋               ×1.046
                            jitless          |██████              ×1.060

 measured move, month 9     jit, box 1       |██▌                 ×1.025
 -> the mid Path B          jit, box 2       |█▎                  ×1.013
 (86e39c65 -> 2856a698)     jitless, box 1   |███▊                ×1.038
                            jitless, box 2   |██▉                 ×1.029

 need left at the mid       jit              |█████████▉          ×1.099-1.101
 (low end to 1.0)           jitless          |██████████▍         ×1.104

 REG-SLOT-ENCODE probe      jit (not read)   |██▌ .. █████        est. +2.5..+5.1% (§9.5)
```

1. **The x86 TS gap is cycles per instruction, not instruction count.** On one box at 86e39c65, ep9 jit and node --jitless ran equal instructions on TS (57.24 G and 57.26 G). The IPC was 2.462 against 2.688. The L1i misses were 0.284 against 0.092 per 1k instructions (3.1×, derived: 0.284 / 0.092). The branch misses were 1.522 against 1.253 per 1k (DL M10-P0-WPRE-1).
2. **Of the 10 rows of §6.1, 2 merged and 6 did not.** W1 is the read itself (done, DL M10-P0-WPRE-1). W5 NAMED-MISS and W10 PREALLOC + BESTFIT merged. W2, W3, W4 and W8 missed their pre-registered lines. W6 is below every line (DL M10-P3-WPRE-3). W7 has no target (DL M10-P4-WPRE-1). W9 became a probe that met its Ir line (DL M10-P4-LEVER14). Two new rows merged: Q1b NAMED-STORE-MEGA and Q9 CALL-FILL (§9.4).
3. **The mid Path B moved the TS cells ×1.013 to ×1.038.** Each cell moved less than the §6.3 middle (×1.046 jit, ×1.060 jitless) (DL M10-P3-WPRE-1).
4. **The low ends still need about ×1.10** (derived: 1 / 0.910 to 1 / 0.906 = ×1.099 to ×1.104). The mid Path B does not have LEVER-13.
5. **The month-10 Ir-to-time factor is about 0.64 (jit) and 1.08 (jitless)** (derived, §9.3). The jitless value is not a pure Ir effect, because the box kernel and the node --jitless times changed too. So the factor stays a range.
6. **REG-SLOT-ENCODE measured -7.923% (jit) and -8.013% (jitless) of x64 TS Ir.** That is more than the R21 estimate -3.4..-5.1% (upper bound; §0, §6.1 W9, §6.3). R21 counted the decode in 4 handlers only (Star, Ldar, Mov and GetNamedProperty, §4). These 4 handlers gave -5.20%. The other T0 handlers (-1.60%), the slow path `R::from_operand` (-0.38%) and the rows not listed (-0.75%) gave the rest (DL M10-P4-LEVER14). Its x86 time is not read.
7. **PGO with a training set that holds no gate workload met no line.** With PGO, jit items 1, 2 and 6 and jitless item 6 (splay) read slower. Jitless item 1 and jitless TS did not read wholly above 1.0. The +3% x86 TS line failed on both builds (DL M10-P0-WPRE-2).
8. **The merged month-10 levers cut x64 TS Ir by 3.61% (jit) and 3.71% (jitless)** (derived, §9.4). Only one merged lever had an x86 box A/B: LEVER-12 NAMED-MISS. Its TS wall did not move on that box (DL M10-P1-B1).

### 9.2 The reads, one row each

| Read | Date | Tree | DL row | Run id or box | Result | R21 estimate it tests | Verdict |
|---|---|---|---|---|---|---|---|
| X86-TOPDOWN-READ | 2026-11-19 | 86e39c65 (the month-9 Path B binaries + a jit build with 64 B function alignment, jit-al) | M10-P0-WPRE-1; M10-PRE-ALIGN-FORM; M10-P1-WPRE-5 | 1 box, 22 min, 31 paired TS rounds | Equal instructions, IPC 2.462 against 2.688 (above). Wall against node: jit 0.901 [0.893-0.910], jitless 0.890 [0.885-0.900]. Top-down level 1, jit against node: retiring 36.3 against 40.2%, backend 23.2 against 19.4%, frontend 21.9 against 24.2%, bad speculation 4.4 against 4.9% (level 2 is not in the microVM). perf record, jit: `ep9h__` handlers 74.1% of the cycles; GetNamedProperty 25.97% (+ _cold 5.01%), SetNamedProperty 5.22% (+ _cold 1.83%), Ldar 4.06%, Star 3.68%, CallProperty1 3.58%, Wide_cold 2.99%; GC trace and sweep about 5.4%. Jitless: CallProperty0 3.26%, Ldar 4.18%. jit-al / jit: wall 0.988 [0.977-0.997], cycles 0.987, instructions 0.999, L1i misses 1.115; 568 of 568 handlers on a 64 B line, against 119 | W1 (§6.1); the §3 IPC reading | Done. The X86-ALIGN condition is met (one box) |
| PGO-PROBE | 2026-11-19 | 86e39c65 | M10-P0-WPRE-2 | 1 box, 41 min, 31 rounds | Training: test262 15,574 tests (13,732 rc 0) and 11 JetStream non-M2 rows (3 scored, 8 errors); 0 profile warnings. t(plain) / t(PGO), jit: item 1 Octane-7 0.980 [0.977-0.990], item 2 v8-v7 0.971 [0.966-0.978], splay 0.981 [0.956-0.986], TS 0.986 [0.972-1.000] (instructions +2.6% with PGO), navier-stokes 0.949. Jitless: item 1 1.000 [0.989-1.005], item 2 0.988, splay 0.962, TS 1.012 [1.000-1.031], richards 1.030 | W8 +3..+10% | NOT TAKEN. The line (+3% x86 TS on both builds) failed on both builds |
| NAMED-MISS-READ | 2026-11-19 | 86e39c65 | M10-PRE-NAMED-MISS-READ | Mac aarch64, an `ic-stats` build; no box | 95.47% of the 41.4 M named misses are megamorphic stub-cache hits in the cold body. More POLY_CASES cuts about 0%. The read counts visits, not time | W5: "the cause is not read" (§6.3) | The cause is read. It became LEVER-12 Q1 |
| WIDE-READ | 2026-11-19 | 86e39c65 | M10-PRE-WIDE-READ | 0 compile | 4.62% of the TS instructions are prefixed (static), about 2% executed (est.). The operand that overflows: the feedback slot 85.9%, the jump offset 12.3%, a register never. A branch island removes est. 5.5-6.7 M prefixed runs | W6 +0..+0.7% | Below every line (DL M10-P3-WPRE-3). No lever |
| DEAD-LDAR probe | 2026-11-19 | probe 2de98538 (base 8165a60e) | M10-P0-DEADLDAR | benches 44434768985, 44434783070 | TS Ir x64 -0.671% (jit) / -0.676% (jitless), a64 -0.636% / -0.642% | W2: TS Ir -1.8..-2.6% from the static 69.87% (§5.1) | Kill line (x64 TS Ir at most -1.0%) NOT MET. Not merged. The dynamic dead share is lower than the static share (§8 K4) |
| LEVER-12 NAMED-MISS | 2026-11-19 | probe 639acc1b; merged as B1 b16ae288 | M10-P0-LEVER12; M10-P1-B1 | TS sets 44449971938, 44479447478, 44524650807; the combined box (27 of 31 rounds) | TS Ir x64: probe -2.192% / -2.265%, after the Codex fixes -2.096% / -2.168%, B0 to B1 -2.098% / -2.167%; a64 (probe) -1.912% / -1.991%. Box B1/B0: TS wall jit 1.000 [0.982-1.014], jitless 0.995 [0.969-1.029] (both straddle 1.0); TS instructions 0.980 / 0.979; splay jit 1.015 [1.006-1.029] | W5 +0..+1.5% | Kill line met; merged in M10-P1. No x86 TS time gain on the box |
| X86-ALIGN form C | 2026-11-19 | B2 2fd411f2 against B1 b16ae288 | M10-P1-X86ALIGN | the combined box (27 of 31 rounds, time cap) | B2/B1: jitless TS 0.987 [0.943-1.006], jit TS 0.995 [0.966-1.008] (both straddle 1.0); jitless cycles 0.976 (wholly below 1.0); t0-branchy jitless 0.917 | W4 jitless +1.7..+2.5% | X86T FAIL. Not merged; the branch is kept for the M2-exit G8 batch |
| REG-SLOT-ENCODE-STUDY | 2026-11-19 | 75b412c4 and 20f953fa | M10-P1-REGSLOT | read only, 0 compile | Form N (a register operand byte = its frame slot offset). Est. x64 TS Ir -3.4..-5.5%, a64 -1.9..-3.2% (upper bound). G8 +2 at a merge | W9 TS Ir -3.4..-5.1% | GO for a probe |
| LEVER-12 NAMED-STORE-MEGA (Q1b) | 2026-11-20 | merged in M10-P2; the merge head 2856a698 | M10-P2-LEVER12; M10-P2-CLOSE-2 | TS sets 44641725575 (probe), 44678132590 (merge head) | TS Ir at the merge head: x64 -0.553% / -0.574%, a64 -0.472% / -0.496%. x64 TS at 2856a698: 55,028,232,423 Ir (jit), 54,417,069,863 (jitless) | not in §6.1 (a new row from W1; est. x64 TS Ir -0.55..-0.73%, DL M10-P1-WPRE-5) | Kill line met; merged. No x86 box |
| STAR-LA2, M10-P2 | 2026-11-20 | c7f185c8 (base 48c0cbcb) | M10-P2-STARLA2 | BENCH 44640863723 | linux-arm64 rows above the +1.0% Ir cap of the time path: t0-branchy +1.546% / +1.556%, richards +1.536% / +1.592%, t0-prop +1.276% / +1.294%. Mac a64 TS jit wall 0.981 [0.971-0.991] | W3 x86 +0..+3% | No X86T box (the Ir cap). Not merged |
| STAR-LA2, M10-P3 | 2026-11-20 | fb55c6e4 (6 sites, 9 of 11 producers) against B3 049ed54d | M10-P3-STARLA2; M10-P3-CLOSE-5; M10-P4-WPRE-5 | the X86T box, 31 rounds, both builds | B/A: jit TS 0.994 [0.983-1.007], jitless TS 0.968 [0.958-0.977]; t0-branchy jit 1.054 [1.025-1.067], jitless 1.065 [1.032-1.091]. TS jitless and t0-branchy jitless at equal instructions (B/A 1.000) | W3 x86 +0..+3% | X86T FAIL (rows wholly above 1.0). Not merged. The gain and the loss are IPC effects |
| The mid Path B | 2026-11-20 | 2856a698 | M10-P2-CLOSE-3; M10-P3-WPRE-1; M10-P3-WPRE-2 | 2 boxes, 58 min each, 31 rounds, both builds | TS jit 0.925 [0.910-0.934] and 0.926 [0.908-0.937], jitless 0.922 [0.906-0.926] and 0.919 [0.906-0.932] (month 9: 0.902, 0.914, 0.888, 0.893). Item 1 jit 1.767 / 1.762, jitless 1.795 / 1.777. Item 2 jit 1.113 / 1.117, jitless 1.111 / 1.109. Item 6 splay jit 1.105 / 1.103, jitless 1.089 / 1.105 (the drop is on the node --jitless side). Raytrace jit 0.947 / 0.961 | the §6.3 middle ×1.046 / ×1.060 | Moves jit ×1.025 and ×1.013, jitless ×1.038 and ×1.029: below the middle |
| WIN-READ | 2026-11-20 | 2856a698 | M10-P3-WINREAD; M10-P4-WPRE-1 | exit-inputs dispatch 44678207050 (GitHub runners) | The pairs w3/l2 and w4/l2 (AMD EPYC 7763). Jitless item 3 low ends 0.915 (w3/l2) and 0.927 (w4/l2); every jit low end at 0.957 or more. Windows TS jit against jitless: the intervals overlap (w3 8.555 s against 8.612 s, w4 8.529 s against 8.481 s). On the GitHub Linux leg l2, jit TS is 3.7% slower than jitless (l1 2.5%) | W7 win-x64 jit +2..+4% | Q3 WIN64-JIT closes NO TARGET |
| The switch row | 2026-11-20 | 2856a698 | M10-P3-SWITCH | the mid Path B | Every linux-x64 TS low end is below 1.0, in both builds. Every linux-x64 TS cell moved less than ×1.04 | §7 (M10-PLAN-D1, -D53) | The switch did not fire; month 11 stays the exit month. The V15-D53 trigger is met: month 12 is the pre-registered fallback |
| LEVER-13 CALL-FILL (Q9) | 2026-11-20 to -21 | merged in M10-P3; the merge head f09c2daa | M10-P3-LEVER13; M10-P3-CLOSE-2 | TS set 44769749909 | TS Ir x64 jit -0.609% (55,029,015,931 to 54,693,906,707), jitless -0.614%; a64 +0.002% / +0.001% (x86-64 only) | not in §6.1 (a new row from W1; est. x64 TS Ir -0.6..-1.1%, DL M10-P3-WPRE-3) | Kill line met; merged. The x86 time is not read (no lever box was left) |
| REG-SLOT probe (LEVER-14) | 2026-11-21 | probe 06ac3354 (P4 24259cf2; base 29ba45f6); a branch only | M10-P4-LEVER14; M10-P4-WPRE-2; M10-P4-WPRE-3 | TS set 44803698999; 19 rows 44803693224 | x64 TS Ir jit 54,692,567,666 to 50,359,216,553 = -7.923%, jitless 54,082,192,731 to 49,748,465,559 = -8.013%; a64 -5.201% / -5.279%. Every one of the 19 rows is lower (x64 jit -3.515% splay to -13.976% t0-loop). Attribution: the 4 study handlers -5.20%, the other T0 handlers -1.60%, the slow path `R::from_operand` -0.38%, the rows not listed -0.75%. The saved Ir per dispatch is an exact integer (x64 Star 3, Ldar 2, GetNamedProperty 3, Mov 6) | W9 TS Ir -3.4..-5.1% (R21); -3.4..-5.5% (the study) | Kill line part 1 (x64 TS Ir at most -2.5%) MET. Not merged in month 10. The x86 time is not read |

### 9.3 The Ir-to-time factor in month 10

| | x64 TS Ir change, 86e39c65 to 2856a698 | x86 TS time change (2-box mean of the medians, month 9 to the mid) | Time per 1% Ir | Month 9 (§2.2) |
|---|---|---|---|---|
| x86 jit | -3.033% (56,749,561,000 to 55,028,232,423) | +1.927% (0.908 to 0.9255) | **0.64** | 0.46 |
| x86 jitless | -3.130% (56,175,245,578 to 54,417,069,863) | +3.369% (0.8905 to 0.9205) | **1.08** | 0.31 |

Derived. The time per 1% Ir = the time change / the Ir drop (1.927 / 3.033 = 0.64; 3.369 / 3.130 = 1.08). Inputs:
- The Ir at 86e39c65: the base rows of run 44303131944 (`findings.json` `ts_ir`). The bench log row (bench log 44303131944 134797714483:1505) reads 56,749,561,000, so this value is the integer, not a rounded value.
- The Ir at 2856a698: TS set 44678132590 (DL M10-P2-CLOSE-2).
- The ratios: the month-9 Path B (§1: 0.902 and 0.914 jit, 0.888 and 0.893 jitless) and the mid Path B (DL M10-P2-CLOSE-3).

The jitless factor above 1.0 is not a pure Ir effect:
- The box kernel changed between the 2 readings, from 6.18.56 to 6.18.64 (DL M10-P3-WPRE-2).
- On the same boxes, the node --jitless time moved ×0.92-0.95 on most rows and ×0.87 on splay. node, QuickJS and the workload texts did not change (DL M10-P3-WPRE-2). So the box moved too. The ratio removes a box move only if the move is the same for both engines.
- Each reading is one box pair.
- An IC fast-body lever read 0 time from -2% instructions on one box: LEVER-12 B1/B0 TS wall 1.000 (jit) and 0.995 (jitless), at TS instructions 0.980 and 0.979 (DL M10-P1-B1).

So the factor stays a range, est. 0 to about 1.0 per lever kind. An Ir number alone does not give the x86 time of a lever. A box A/B in time reads it (the form of the rule 13 time path, M10-PLAN-D7).

### 9.4 The lever list as built

| # | Lever | §6.1 estimate | Month-10 result | Merged | G8 |
|---|---|---|---|---|---|
| W1 | X86-TOPDOWN-READ | steering | Done: the gap is cycles per instruction (DL M10-P0-WPRE-1) | a read | 0 |
| W2 | DEAD-LDAR | +0.6..+1.8% (x64 TS Ir -1.8..-2.6%) | x64 TS Ir -0.671% / -0.676%; the kill line -1.0% is not met (DL M10-P0-DEADLDAR) | no; the branch is kept | 0 (the H24 R4 rewrite is not spent) |
| W3 | STAR-LA2 | x86 +0..+3% | M10-P2: Ir cap fail; M10-P3: X86T FAIL, jitless TS 0.968 but t0-branchy jitless 1.065 (DL M10-P2-STARLA2, DL M10-P3-STARLA2) | no | 0 |
| W4 | X86-ALIGN | jitless +1.7..+2.5%; jit 0..+1.5% | 0.988 on the X86-TOPDOWN-READ box (jit); X86T FAIL on the combined box, jitless 0.987 [0.943-1.006] (DL M10-P1-X86ALIGN) | no; the branch is kept | 0 (the +1 is not spent) |
| W5 | NAMED-MISS (LEVER-12) | +0..+1.5% | x64 TS Ir -2.098% / -2.167% (B0 to B1); box TS wall 1.000 / 0.995, both straddle 1.0 (DL M10-P1-B1) | yes, M10-P1 (B1 b16ae288) | 0 |
| W6 | WIDE-READ -> WIDE-CUT | +0..+0.7% | Read only; the only G8-0 form (a branch island) is below every line (DL M10-PRE-WIDE-READ, DL M10-P3-WPRE-3) | no | 0 |
| W7 | WIN64-JIT | win-x64 jit +2..+4% | No target: the Windows jit and jitless TS intervals overlap (DL M10-P4-WPRE-1) | no | 0 |
| W8 | PGO-PROBE | +3..+10% | TS jit 0.986, jitless 1.012; the +3% line failed (DL M10-P0-WPRE-2) | no (NOT TAKEN) | 0 |
| W9 | REG-SLOT-ENCODE (LEVER-14) | TS Ir -3.4..-5.1% (upper bound) | Probe: x64 TS Ir -7.923% / -8.013%, kill line part 1 met; x86 time not read (DL M10-P4-LEVER14) | no (a probe branch; a month-11 merge role, DL M10-P4-WPRE-2) | 0 in month 10; +2 at a month-11 merge (DL M10-P4-WPRE-3) |
| W10 | PREALLOC + BESTFIT | +0.1..+0.2% (about -0.4% TS Ir) | x64 TS Ir -0.217% / -0.219% (PREALLOC), -0.178% / -0.179% (BESTFIT) (DL M10-P0-BASE; §6.1 W10) | yes, the M10-P0 base | 0 |
| Q1b | NAMED-STORE-MEGA (LEVER-12) | not in §6.1 (est. x64 TS Ir -0.55..-0.73%, DL M10-P1-WPRE-5) | x64 TS Ir -0.553% / -0.574%; no x86 box (DL M10-P2-CLOSE-2) | yes, M10-P2 | 0 |
| Q9 | CALL-FILL (LEVER-13) | not in §6.1 (est. x64 TS Ir -0.6..-1.1%, DL M10-P3-WPRE-3) | x64 TS Ir -0.609% / -0.614%, a64 +0.002% / +0.001%; no x86 box (DL M10-P3-CLOSE-2) | yes, M10-P3 | 0 |

Month-10 G8: 0 of +2 (DL M10-P4-G8). ABI_VERSION stays 6 on the main line all month; 7 is on the probe branch only (DL M10-P4-WPRE-3, DL M10-P4-G8).

**The sum.**
- The 5 merged rows (W10 PREALLOC, W10 BESTFIT, W5, Q1b, Q9) give x64 TS Ir -3.614% (jit) and -3.710% (jitless) (derived: the product of (1 + change) over the 5 rows above).
- The direct read from 86e39c65 to the `m10-p4` code gives -3.625% and -3.726% (derived: 56,749,561,000 to 54,692,567,666 and 56,175,245,578 to 54,082,192,731; the end values are the base rows of TS set 44803698999 on 29ba45f6, DL M10-P4-LEVER14). The rest, -0.011 and -0.016 points (derived), is the other base merges (for example the M10-P0 merge head, x64 TS -0.010%, TS set 44499111404, DL M10-P1-B1; the HASH-SEED-2 head, -0.000% / -0.001%, DL M10-P1-CLOSE-2) and the difference between runs.
- The measured time move at the mid (2856a698, without LEVER-13): jit ×1.013 and ×1.025, jitless ×1.029 and ×1.038 (§9.2).
- The rest for month 11 is the low-end need at the mid, about ×1.10 (§9.1 point 4). The month-end Path B adds the LEVER-13 reading to it.

### 9.5 What the reads say for the next levers

These are facts and derived estimates. The plan writer decides.
- **REG-SLOT-ENCODE: the x86 time is not measured.** Kill line part 2 (one x86 box A/B, 31 paired rounds, both builds, TS wall median at most 0.990 and the interval wholly below 1.000) is on the first month-11 lever box (DL M10-P4-WPRE-2). Est. x86 TS time from -7.923% / -8.013% Ir with the month-9 factors 0.31-0.46: +2.5..+3.7% (derived: 7.923 × 0.31 = 2.46; 8.013 × 0.46 = 3.69). With the month-10 jit factor 0.64: about +5% (derived, est.: 7.923 × 0.64 = 5.07).
- **STAR-LA2: the gain and the loss came at equal instructions.** Jitless TS 0.968 and t0-branchy jitless 1.065 at B/A instructions 1.000 (DL M10-P4-WPRE-5). They are IPC effects (DL M10-P3-CLOSE-5). So a narrower form needs a cause read first; DL M10-P4-WPRE-5 pre-registers a 0-code cause read.
- **X86-ALIGN: one box gave a gain, the next box did not.** jit-al / jit 0.988 [0.977-0.997] on the X86-TOPDOWN-READ box (DL M10-PRE-ALIGN-FORM). B2/B1 on the combined box straddled 1.0 in both builds, with 27 of 31 rounds read (DL M10-P1-X86ALIGN). That row keeps the branch for a retry with more rounds.
- **The switch did not fire, and the V15-D53 trigger is met.** Month 12 is the pre-registered fallback exit month (DL M10-P3-SWITCH).
- **The X86-TOPDOWN-READ hot list has a lever for each function at 3% or more of the cycles** (DL M10-P1-WPRE-5): GetNamedProperty (W5), SetNamedProperty (Q1b), CallProperty1 and CallProperty0 (Q9), Star (W3) and Ldar (W2). Of these levers, W2 and W3 did not merge.
- **jit against jitless depends on the host.** On the GitHub Linux legs, jit TS read 2.5% (l1) and 3.7% (l2) slower than jitless (DL M10-P4-WPRE-1). On the mid Path B boxes, the jit TS ratio was 0.3% and 0.8% above the jitless one (derived: 0.925 / 0.922 = 1.003; 0.926 / 0.919 = 1.008; DL M10-P3-WPRE-1). DL M10-P4-WPRE-1 sorts the GitHub Linux-leg gap as a reading-form item, not a lever; the void-leg rule is DL M10-P4-WPRE-6.

### 9.6 Limits

| # | Risk | Effect | What reads it next |
|---|---|---|---|
| K8 | One box or one box pair per reading (X86-TOPDOWN-READ, PGO-PROBE, the combined box and the STAR-LA2 box: 1 box each; the mid Path B: 1 pair) | A time effect of 1-2% can change sides between boxes. X86-ALIGN read 0.988 on one box and straddled 1.0 on the next | Each line was set before its reading (rule 86). The X86-ALIGN branch is kept for a retry with more rounds (DL M10-P1-X86ALIGN) |
| K9 | The box kernel changed between month 9 and the mid Path B (6.18.56 to 6.18.64) | The §9.3 factor holds a box move that the ratio does not fully remove (node --jitless ×0.92-0.95, splay ×0.87) | DL M10-P3-WPRE-2 read the per-engine medians. The month-end Path B reads the cells again |
| K10 | The mid Path B is not the month-end reading | It has no LEVER-13 (2856a698 is before f09c2daa) | The month-end Path B on the `m10-p5` tag; it is in the month-10 report, not here |
| K11 | LEVER-13 and REG-SLOT-ENCODE have no x86 time reading | §9.4 counts them in Ir only; with a factor of 0 to about 1.0 their time is open | LEVER-13: the month-end Path B (DL M10-P3-LEVER13). REG-SLOT-ENCODE: kill line part 2 on the first month-11 lever box (DL M10-P4-WPRE-2) |
| K12 | WIN-READ has one pair kind only (AMD EPYC 7763), and the paired leg l2 has void kernel rows | Jitless item 1 and jit item 2 read NO AGREEMENT on the 09:66 form (DL M10-P3-WINREAD) | The void-leg rule, option A (DL M10-P4-WPRE-6) |
| K13 | PGO used one training set | Another set with no gate workload can read differently | A set with a gate workload is not allowed (DL M10-P0-WPRE-2; §8 K7) |

## Raw numbers

[artifacts/x86-ts-time/](artifacts/x86-ts-time/README.md) holds `findings.json` (the numbers of §1-§6) and `month10.json` (the numbers of §9), each with its sources.
