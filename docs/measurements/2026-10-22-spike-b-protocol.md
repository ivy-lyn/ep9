# Spike B round-trip protocol (the D21 gate), written before any spike code

- **Status.** Protocol only; no result. Written in M0 month 2, phase P0 (2026-10-22), before any Spike B code exists (decision M2-D21-METHOD: a gate written first cannot be tuned to the result). A change after the first gate run needs a decisions-log row that names this file.
- **Gate.** D21 keeps native-stack JIT frames (placement (a)) if one T0→JIT→T0 round trip costs ≤ 18 release-expansion dispatches of the SP-Dispatch loop kernel, on the same host and in the same session ([09 §4](../plan/09-milestones.md#4-spikes) Spike B row; decisions-log D21 and D21-UNIT).
- **Class.** S (spike) of the speed-gate rule ([10-quality §3](../plan/10-quality.md), PERF-NOISE): wall time of the timed region, ≥ 15 paired rounds, order-statistic interval.
- **Who runs what.** P3-SPIKEB-BENCH builds `spikes/spike-b/bench.sh` and does a provisional run. The gate run is the orchestrator's gate window 1, after the P3 merge, with no agent running.

## 1. The formula

```text
RT_disp = ((t_A - t_C) / n) / tau - k

t_A, t_C  wall time of the timed region of kernel A and kernel C in one round (ns)
n         loop iterations of each kernel (the same n for A and C)
tau       ns per dispatch of the SP-Dispatch loop kernel, release expansion, same host and session
k         extra T0 dispatches per iteration of A over C, not counting the call instruction itself
```

So RT_disp counts the call instruction, the JIT callee and the return to T0, in units of one loop-kernel dispatch.

## 2. Kernels A and C

Both run in one release build of `spikes/spike-b` (feature `spike-native`). Only `id` is compiled by the spike JIT; its bytecode is `Ldar a0; Return`, two of the 10 Spike B opcodes. `drive` always runs in T0.

```js
// kernel A                                       // kernel C: the call statement removed
function id(x) { return x; }                      function id(x) { return x; }
function drive(f, n) {                            function drive(f, n) {
  let a = 0;                                        let a = 0;
  for (let i = 0; i < n; i++) { a = f(a); a = a + 1; }   for (let i = 0; i < n; i++) { a = a + 1; }
  return a;                                         return a;
}                                                 }
```

- n = 100,000,000. Every value stays an int32 (a ≤ n < 2^31), so no double path runs.
- A run is valid only if `drive` returns n. A wrong result fails the run, not the gate.
- Configurations of one round:

| Config | `drive` | `id` | Use |
|---|---|---|---|
| A | T0 | JIT (Baseline slot) | gate |
| C | T0 | not called | gate |
| A0 | T0 | T0 | report: T0→T0 call, same formula and k |
| J, JC | JIT | T0 (J); not called (JC) | report: JIT→T0→JIT round trip, `(t_J - t_JC) / n` in ns and in tau units |

## 3. tau

- Run `bash spikes/sp-dispatch/gate.sh` (SP_SPEED=gate, SP_ROUNDS=15) first in the same session. It writes `spikes/sp-dispatch/target/gate/bench.json`.
- tau = `median_ns.sibling / dispatches` of the kernel named `loop` in that file. Check that `release` is `sibling` (H15) and `scale` is 1 (then `dispatches` is 880,000,017).
- The bench also prints the order-statistic interval of the per-round sibling loop times divided by `dispatches`, as a report.
- "Same session" means: one quiet window, one boot, the same power mode, no build or other benchmark between the SP-Dispatch run and the Spike B run.

## 4. k

- Take the `ep9 --print-bytecode` listing of A and C from the same build. In `drive`, count the instructions from the `JumpLoop` target to the `JumpLoop` itself. Each of them runs once per iteration (the only branch in the range is the loop exit). k = (count in A) − (count in C) − 1.
- A wide or extra-wide prefix in either loop range fails the run: in T0 a prefix takes the cold path, so it is not one dispatch.
- The bench prints the counted lines of both listings.

**Example at 55c15d0 (ABI 2).** A's loop (offsets 6-40) has 16 instructions; C's loop (offsets 6-26) has 10. The extra ones in A are `Ldar a0`, `Star r2`, `Ldar r0`, `Star r3`, `CallUndefinedReceiver1 r2, r3, s1` and `Star r0`, so k = 16 − 10 − 1 = 5.

```text
A:  6 Ldar a1 | TestLessThan r1 | JumpIfFalse | Ldar a0 | Star r2 | Ldar r0 | Star r3 |
    CallUndefinedReceiver1 r2, r3 | Star r0 | Ldar r0 | AddSmi [1] | Star r0 | Ldar r1 | Inc |
    Star r1 | 40 JumpLoop @6
C:  6 Ldar a1 | TestLessThan r1 | JumpIfFalse | Ldar r0 | AddSmi [1] | Star r0 | Ldar r1 | Inc |
    Star r1 | 26 JumpLoop @6
```

## 5. One run, one round

- **One process per run.** The process makes the isolate, compiles and loads the kernel, installs the JIT code (A, J), and calls `drive(f, 10000)` once, untimed. Then it times one call `drive(f, n)` from Rust with a monotonic clock around the call only. Process start, compile and JIT time are outside the timed region.
- **Round.** Each round runs every config once, back to back. Round r starts at config `r mod m` of the list (rotated order). Round 0 is a warm-up and is dropped.
- **Rounds.** 15 kept rounds (16 runs per config). The re-run of a marginal result uses 31 kept rounds (at least 2x).

## 6. Statistic and verdict

- Per kept round r: `RT_r = ((t_A,r - t_C,r) / n) / tau - k`. The gate value is the median of RT_r.
- The 95% interval of the median comes from order statistics: 15 rounds → ranks 4 and 12 (96.5%); 31 rounds → ranks 10 and 22 (97.1%).

| Verdict | Rule |
|---|---|
| pass | the whole interval is ≤ 18 |
| fail | the whole interval is > 18 |
| marginal | otherwise: run again with 31 rounds; the median then decides, and the result keeps the label "marginal" |
| void | the IQR of t_A or of t_C is > 5% of its median: the host was not quiet; run again |
| provisional | the load guard failed, or the host is not the gate host: no pass and no fail |

## 7. Host and load guard

- Gate host: `m5max-macos` (`tools/bench/hosts/m5max-macos.toml`), in a quiet window that the orchestrator runs alone. macOS: no core pinning.
- Load guard ([10-quality §3](../plan/10-quality.md)): before the run, after 60 s with no benchmark process, and 120 s after it, the 1-minute load must be ≤ idle baseline + 0.5 = 2.505 + 0.5 = **3.005**. If the check before fails, wait up to 30 min, then stop with "host busy". Use `guardBefore` and `guardAfter` of `tools/bench/lib/host.mjs`. The result records both values.

## 8. Reports (never a gate)

- A0: the T0→T0 call in tau units (same k). J: the JIT→T0→JIT round trip.
- Instruction ratio with cachegrind (`valgrind --tool=cachegrind --smc-check=all`, needed for JIT code): `((Ir_A - Ir_C) / n) / (Ir_loop / dispatches) - k`, with `Ir_loop` from `sp-dispatch run sibling loop.ep9d` (the file comes from `sp-dispatch dump loop default loop.ep9d`) in the same build mode. Process start-up is inside `Ir_loop`; the bench prints its size (the Ir of `sp-dispatch ops`) next to the ratio. n may be 1,000,000 under cachegrind. Linux arm64 in Docker by the agent; x86-64 on the sandbox by the orchestrator. Each row names its CPU model.

## 9. Output and what follows

- `bash spikes/spike-b/bench.sh` prints: host, CPU model, commit, load before and after, tau with its source file, k with the counted listing lines, every round's t_A, t_C and RT_r, the median, the interval, the verdict and its labels. The result goes to `docs/measurements/<date>-spike-b.md`.
- **pass:** placement (a) stays; SP-Unit starts in P4.
- **fail:** placement (b) (JIT frames on the VM stack, entered with the 4 handler arguments, in `spikes/spike-b/src/jit_b/`) is built and measured with this same protocol (M2-D21-FAIL). D21 reopens only if both placements fail.
- **Sizing note.** n was chosen so that t_C is about 0.3 s: at 55c15d0, `ep9` ran kernel C with n = 2·10^7 in 0.06 s of process wall time (2026-10-22, load 18.7, so a sizing figure only).
