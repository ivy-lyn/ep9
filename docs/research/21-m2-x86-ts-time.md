# x86 TS time for the M2 gate: the need per cell, the Ir-to-time factor, the real counters, the V8 forms and the month-10 levers

## Run context

| Item | Value |
|---|---|
| Date | 2026-11-19 |
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

## 9. Addendum (to come)

Before plan v16, the orchestrator adds the month-10 reads to this report (M10-PLAN-D52; plan v15 §13.4 step 7): X86-TOPDOWN-READ, NAMED-MISS-READ, WIDE-READ, PGO-PROBE, REG-SLOT-ENCODE-STUDY and the mid Path B.

## Raw numbers

[artifacts/x86-ts-time/](artifacts/x86-ts-time/README.md) holds `findings.json`: the numbers of §1-§6 with their sources.
