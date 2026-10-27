# Spike C protocol (the Spike C gate and the SP-Facts compare), written before any compiler code

- **Status.** Protocol. Written in M1 month 1 (month 4), phase M4-P0, on 2026-10-26 (the file name keeps the date the plan gave it), before the Spike C compiler exists: `spikes/spike-c` has the harness, the kernel drivers and the facts input; the IR, the graph builder, register allocation and the a64 emitter are stubs (`// m4-stub`). A gate written first cannot be tuned to the result (the Spike B rule, M2-D21-METHOD). A change after the first gate run needs a decisions-log row that names this file.
- **Gate.** The Spike C row of [09 §4](../plan/09-milestones.md#4-spikes) (09-milestones.md:252), M1 exit items 7 and 10 (09-milestones.md:96, :99), the A0 gate (09-milestones.md:210) and the A0 time box (09-milestones.md:238-241).
- **Decisions applied.** Plan v9 V9-D32 (log id M4-PLAN-D32), V9-A23 (M4-PLAN-A23) and the orchestrator decision M4-PLAN-ORCH-1: **the whole-kernel ratio gates "≥ 3x T0"**; the compiled hot function in a class S harness is a context line. V9-A20: Spike C reads named and int facts first; keyed facts come from FACTS2 (M4-P2), so they enter in SPIKEC-3.
- **Class.** S (spike) of the speed-gate rule ([10-quality §3](../plan/10-quality.md), PERF-NOISE): wall time of the timed region, ≥ 15 paired rounds plus 1 warm-up round dropped, the order-statistic interval of the median.
- **Who runs what.** The SPIKEC roles (one JIT/platform role chain: P0-SPIKEC, SPIKEC-1 … SPIKEC-4 in month 4, then months 5-6) build and report. The gate runs and the Verdict rows are the orchestrator's (plan v9 rule 3), in a quiet gate-host window with no agent running.

## 0. The conditions, one row each

| # | Condition (09-milestones.md:252 and the A0 gate) | Where here | Pass rule | Built in |
|---|---|---|---|---|
| C1 | "Small SSA compiler for 2-3 v8-v7 kernels on arm64" | §1, §2 | the hot set of 2 gate kernels compiles from live facts on aarch64 and runs with output equal to T0 (for navier-stokes the check line holds a digest of every density and velocity cell in index order, §2); a third kernel is named (§2.3) | M4 (crypto), M5 (navier-stokes) |
| C2 | "One eager deopt from a native frame to a VM-stack frame" | §4 | a forced guard failure in compiled code resumes T0 at the exit's bytecode offset in the same VM frame; output equal, check line included (§2); the exit counter counts it | M5 (the deopt entry in `ep9_vm`, behind `spike-native`) |
| C3 | "Deopt correct under the tier-differential test" | §5 | 0 output mismatches between T0 and Spike C over the kernels and the seeded corpus (the navier-stokes check line holds the cell digests, so a moved or changed cell is a mismatch), with every guard forced to fail at least once | M5 |
| C4 | "Kernels ≥ 3x faster than T0 (est.)" | §3 | the **whole-kernel** ratio t(T0) / t(Spike C) ≥ 3.0 for each gate kernel, class S verdict (the interval on the pass side), and every gate-window run right (§3: one wrong run fails the gate); the hot-function ratio is reported, never gated | M6 gate window |
| C5a | SP-Facts: "the Spike C IR uses the fact lattice and ref nodes" | §6.1 | structural checks on every compiled graph | M4-M5 |
| C5b | SP-Facts: "its deopt metadata is position-independent" | §6.2 | the metadata holds offsets only; a moved copy of code + metadata deopts the same | M5 |
| C5c | SP-Facts: "one kernel compiled from a PROFILED FactSnapshot file gives identical output at ≥ 90% of live-feedback speed (est.)" | §6.3 | identical output; t(live) / t(PROFILED) ≥ 0.90, class S; after the time box a speed-only miss is recorded in H30, not blocking (09-milestones.md:241) | M5-M6 |
| R | Reopens on fail: H2, H14, H16, H17, H29; SP-Facts: H30-H34 | §8 | a correctness or structure failure reopens the named ADRs | - |

## 1. Scope and rules

- **Arm64 only** (09-milestones.md:252). On any other architecture the pipeline refuses with `Refusal::Arch` and the function stays in T0 (`spikes/spike-c/src/lib.rs`, `compile`).
- **Throwaway, behind `spike-native`.** The spike is its own workspace (`spikes/spike-c/Cargo.toml`, own lock), with path dependencies on `crates/ep9_vm` (feature `spike-native`) and on Spike B's lib. `spike-native` is never in the ep9 graph (`.github/scripts/dep-guards.sh` check 11), and no JIT tier code runs JS before the M2 gate (09-milestones.md:36).
- **Placement (b).** Compiled code is installed in an Opt slot with `ep9_vm::install_native_b` (`spike_c::harness::Session::install_b`): the frame is the canonical VM frame that T0 pushed, the code is entered with the T0 handler ABI and leaves by a tail jump to `cx.table[*pc]` (H14 "JIT frames, placement (b)"). T0 enters (b) code only under the threaded dispatch (`Dispatch::Sibling`, the release default); the debug match loop runs the bytecode instead.
- **One graph builder for both fact paths** (H30 rule; V9-D32): the builder reads bytecode plus one FactSnapshot. The live path takes `ep9_vm::abi::export_facts` output (`spike_c::facts::live`, source RUNTIME); SP-Facts takes a PROFILED file (`spike_c::facts::read_profiled`). The builder never reads a FeedbackVector.
- **Guard policy, JIT mode** (H32 table): PROVEN facts get no guard; a RUNTIME, PROFILED or HINTED fact gets a guard whose failure is an eager deopt plus an exit counter. Until month 5 a guard exit calls a "deopt not built" trap (SPIKEC-1).
- **Liveness** is computed by the graph builder (H02-Q1, V9-D32); the builder reports its time and the size it would take as a section, for the H2 freeze.
- **Deopt entry** in `ep9_vm` behind `spike-native` (H29: "Deoptimizer runtime in `ep9_vm`"); `DEOPT_META_VERSION` stays 0 (no shipped metadata). Lazy deopt is a time-boxed probe in month 6 (V9-D32); it is not a 09 condition.

## 2. Kernels

All files are the pinned v8-v7 files (`third_party/bench-v8v7.commit`, `bench/bench-v8/`): the driver refuses a file whose git blob id is not the one at the pin (`spike_c::kernels::PINNED`), a `SPIKE_C_BENCH_DIR` copy included. The driver is the stub harness of `tools/bench/cross/v8v7.mjs` (decision PERF-2): the kernel file as it is, a stub `BenchmarkSuite`/`Benchmark`, base.js's deterministic `Math.random`, then Setup, the hook, `run()` n times, the check line, TearDown. `spike-c driver <kernel> [--n N]` prints the text; node runs the same text (`spike-c run <file.js>` runs any script in T0 on the same harness).

| Kernel | Role | Hot set (file lines) | Check |
|---|---|---|---|
| crypto | gate kernel 1 | `am3` (crypto.js:101-115), `montReduce` (:576-593), `bnpSquareTo` (:424-441) | the kernel throws on a wrong decrypt (crypto.js:1689) |
| navier-stokes | gate kernel 2 | `lin_solve` (navier-stokes.js:122-149), `project` (:233-269), `advect` (:198-231) | the v7 file has no check: after the `run()` loop the driver prints `checksum <d> <u> <v> digest <ha> <hb>` over every cell of the last frame's density, x velocity and y velocity fields, boundary cells included, in index order: the three sums, and two exact 32-bit digests (FNV-1a and a polynomial hash mod 4294967291) of each cell's number text, so a cell's value and position both count (`kernels::NS_CHECK`). A cell that is not a finite number makes the line `checksum bad <count>`, never a pass; -0 is kept apart from 0. T0 equals node v24.22.1 (test `navier_stokes_checksum_equals_node`). A run without the check line, or with a line not of this form, is wrong; at n = 1 the line must equal node's, and for any n it must equal the T0 run's (`Driver::matches`, `Driver::matches_t0`). Negative controls: swapping the unequal density cells (58,0) and (59,0) keeps the sums but fails; so do -0 in a 0 cell and a NaN, Infinity, `undefined` or string cell (test `navier_stokes_check_sees_each_cell`) |
| richards | third candidate | `Scheduler.prototype.schedule` (richards.js:183-193), `TaskControlBlock.prototype.run` (:319-333) | the kernel throws on wrong counts |

The first compile target is `am3` (SPIKEC-1). The other hot functions follow (the plan names `lin_solve` for SPIKEC-4). The plan text cites am3 as crypto.js:101-114 and lin_solve as navier-stokes.js:122-150; the functions end on lines 115 and 149.

### 2.1 Hot-set shares and what they mean for C4

Self-time share of each function under `node --jitless --cpu-prof` (the closest engine to T0 that has a per-function profiler; ep9 has none), 2026-10-26, node v24.22.1, the same driver text, 3-4 samples per kernel at sampling intervals of 1000, 200 and 100 µs. The host load was 18-31, so the shares are **(est.)** and only show the shape.

| Kernel | Function | Share (min-max) |
|---|---|---|
| crypto | am3 | 48.1-56.1% |
| | montReduce | 21.8-28.4% |
| | bnpSquareTo | 5.2-11.2% |
| navier-stokes | lin_solve | 43.1-51.5% |
| | project | 15.5-23.2% |
| | advect | 5.5-11.9% |
| richards | Scheduler.prototype.schedule | 34.7-48.2% |
| | TaskControlBlock.prototype.run | 12.0-14.2% |
| raytrace | rayTrace (the top function) | 16.2-20.0% |

The whole-kernel ratio is bounded by the share p of the compiled set: with that set s times faster, R = 1 / ((1 - p) + p / s), so R < 1 / (1 - p) for any s.

```text
 compiled set                         p (est.)     R if s = ∞     s needed for R = 3
 ──────────────────────────────────── ──────────── ────────────── ──────────────────
 crypto: am3 alone                    48-56%       1.9-2.3x       not reachable
 crypto: am3 + montReduce             70-85%       3.3-6.5x       4.7-21.6x
 crypto: + bnpSquareTo                75-96%       4.0-23x        3.3-8.9x
 navier-stokes: lin_solve alone       43-52%       1.8-2.1x       not reachable
 navier-stokes: + project + advect    64-81%       2.8-5.4x       5.5x to not reachable
 richards: schedule + TCB.run         47-62%       1.9-2.7x       not reachable
```

So: **one compiled function per kernel cannot pass C4**; the gate needs the hot set of §2, each function several times faster than T0 (est.). This is a fact of the gate unit that M4-PLAN-ORCH-1 chose, not a reason to change it. The protocol reports, for each gate run, the implied T0 share of the compiled set from the two measured ratios (§3): p = (1 - 1/R) / (1 - 1/s).

**Note (SPIKEC-2, M4-P2): the node shares are too low for ep9 T0 on crypto.** The SPIKEC-2 Mac report (`gate.sh time`, a report and never a reading; host load 28.6 from other agents, so void by the IQR rule: IQR 6.7%) gives crypto R = 4.065, interval [4.018, 4.475], with am3 alone compiled (s = 6.763, interval [6.672, 6.926]). The implied share is p = 88.5%: ep9 T0 spends about 88% of crypto's time in am3 (est., loaded host), where node --jitless spends 48.1%. An earlier 6-round run at load about 8 gave R = 4.000 [3.888, 4.103]. So on crypto one compiled function can reach R ≈ 4.0 on the Mac (est.), and the sentence "one compiled function per kernel cannot pass C4" holds for the node shares only. The gate still measures the whole kernel with the §2 hot set on the gate host (§3); the other kernels have no ep9 share yet.

### 2.2 Facts at the M4-P0 base (`spike-c facts <kernel>`, d7cc797, n = 1)

| Function | Live records | Kinds |
|---|---|---|
| am3 | 2 | LoadNamed (`this.array`, `w.array`) |
| montReduce | 27 | LoadGlobal, LoadNamed, StoreNamed |
| lin_solve | 0 | (context reads and keyed accesses: no record at the base; 46 live records after FACTS2 and KEYED, SPIKEC-4 at e776784) |
| project | 2 | LoadGlobal, LoadNamed |
| advect | 0 | |
| Scheduler.prototype.schedule | 14 | LoadNamed, StoreNamed |
| TaskControlBlock.prototype.run | 12 | LoadGlobal, LoadNamed, StoreNamed |

T0 records no BinaryOp, CompareOp, keyed or Call facts at the base (`crates/ep9_vm/src/facts_export.rs:9-12`). The int facts of `am3` are PROVEN from `&` and `>>` (H30 PROVEN row); keyed and number facts come from PROFILE (M4-P1) and FACTS2 (M4-P2).

### 2.3 The third candidate: richards, not raytrace

The plan default was raytrace (est.). Read in the pinned files and profiled as in §2.1, raytrace has a flat profile: its top function `rayTrace` (raytrace.js:703) has 16-20%, and the next ones are the `Class.create` constructor wrapper that calls `this.initialize.apply(this, arguments)` (raytrace.js:28-32), the `initialize` methods and small `Vector` methods. A small SSA compiler with no inlining gains little there. Raytrace also does not run in T0 at the base: `Math.pow` at raytrace.js:792 throws "not implemented in ep9 v0: ** with a result that is not an exact integer" (the H36 pow core; MATH1 builds it in M4-P1). Richards runs today, checks itself, and its top two functions hold 47-62%. So the third candidate is **richards, `Scheduler.prototype.schedule` then `TaskControlBlock.prototype.run`**; raytrace stays a named fallback. Both are property and call heavy, so a third kernel at ≥ 3x needs more than two functions (§2.1); it is a fallback for C1 ("2-3 kernels"), and a drop at month 6 is allowed with a row only if 2 kernels remain (V9-A23).

## 3. Speed (C4)

Both configurations run in one release build of `spikes/spike-c` (panic = "abort", the T0 release dispatch), on the same driver text.

| Config | What runs | Use |
|---|---|---|
| T0 | the driver, nothing installed | the baseline |
| C | the same driver; after the warm-up the hot set of §2 is compiled from the live facts of that run and installed (placement (b)) | **gate** (whole-kernel ratio R = t(T0) / t(C)) |
| H0, H1 | the first hot function alone in a class S harness with fixed inputs, in T0 (H0) and compiled (H1) | context: the hot-function ratio s = t(H0) / t(H1) (JA-D1) |
| P | as C, with the facts from a PROFILED file (§6.3) | SP-Facts speed (C5c) |

- **One run.** A new process per config and round. Script 1: the kernel file, Setup and one warm-up `run()` (the feedback). Then config C (or P) compiles and installs. Script 2, the timed region: `run()` n times (n of `tools/bench/cross/v8v7.mjs`: crypto 8, navier-stokes 12, richards 200). The timer stops; then the check line (for navier-stokes the cell digests of §2), outside the timed region.
- **Correctness is part of the gate.** Every run of config C (or P), the warm-up round included, must print the T0 output, check line included (`Driver::matches_t0`; at n = 1 the navier-stokes line also equals node's) and end with no uncaught error and no crash. One wrong output, uncaught error or crash in any run fails the gate as a correctness failure (C1 for config C, the identical-output rule of C5c for config P; §8: the named ADRs reopen). Such a run is never dropped, and rounds are not run again to get past it. Only host reasons void a set of rounds (the load guard of 10-quality §3, the IQR rule below); a void set runs again whole, and its wrong runs still count.
- **Round.** T0 and C (and H0/H1, P when measured) once each, back to back, in rotated order. Statistic: the per-round ratio. ≥ 15 rounds plus 1 warm-up round dropped.
- **Verdict.** Gate value = median of the paired ratios R. Interval of the median by order statistics (n = 15: ranks 4 and 12). Pass: the whole interval ≥ 3.0. Fail: the whole interval < 3.0. Otherwise marginal: once more with 2x rounds; the median decides and the label "marginal" stays. Void: IQR of the ratios > 5% of the median; run again.
- **Host.** The gate host (macOS arm64 in a quiet window, load guard of 10-quality §3), with no agent running. A Mac timing by a SPIKEC role is a report (SPIKEC-2, SPIKEC-3), never a reading. The result names the CPU model.
- **Reports next to the gate value:** s (H0/H1), the implied share p (§2.1), the code size of each compiled function, the guard count, and the exits taken during the timed region (expected 0).
- **Harness (SPIKEC-2, M4-P2).** `spike-c time <kernel>` (one process per config through `spike-c once`; `spikes/spike-c/src/timing.rs`), run by `gate.sh time`, is the class S harness of this section: configs T0, C, H0 and H1 in new processes, in an order rotated by the round, ≥ 15 rounds plus 1 dropped warm-up, the median of the paired ratios, the order-statistic interval, the IQR rule and the implied share p. Every run must be right (else the report is a correctness failure), and the check line runs after the timer stops. `gate.sh time` is not part of `gate.sh all`. On the Mac it gives a report only (Host, above).

## 4. Eager deopt (C2)

"From a native frame to a VM-stack frame": under placement (b) the compiled function's frame is already the canonical VM frame (H14). What lives in native state are the values in machine registers (and the native frame record while the body runs). The deopt writes them back into that VM frame and resumes T0 there.

```text
 guard fails ──> exit stub <i> ──> shared deopt entry (ep9_vm, spike-native; month 5)
   metadata of exit i (by pc offset from the code start): bytecode offset, value locations
   (reg / stack / const-index) of every live register and the accumulator, exit-counter index
   ──> write each live value into its VM register slot, acc into the acc register
   ──> exit counter (RW array of the compiled function) += 1
   ──> resume T0: tail jump to cx.table[*pc] with pc = bytecode start + offset (as RuntimeTable::unwind does)
```

- H29-Q7: the entry allocates nothing for the value kinds the spike uses (int32, double, object references), so it is not a GC point. A kind that needs materializing is refused at compile time.
- After a deopt the function stays installed (JIT mode, H32: DEOPT + exit counter). No re-tier policy is gated.
- **Pass:** for each gate kernel, a run with one forced failure of each guard (§5 forcing) prints the same output as T0, every exit counter equals the number of forced failures of its guards, and the heap verifier is clean after the run; the same under GC stress (`Config::stress`) with n = 1.

## 5. Tier-differential test (C3)

Method (V9-D32, JA-D2): a seeded corpus plus forced deopts in the spike; Fuzzilli with `--force-tier` stays at M5 (the 10-quality §4 row "Tier differential + deopt").

- **Forcing.** With `SPIKE_C_FORCE_DEOPT=<seed>`, guard g of a compiled function fails on its k-th execution, k = 1 + (splitmix64(seed XOR g) mod 1000) (est.), once per run. Without the variable no guard is forced.
- **Corpus.** (a) The gate kernels with their checks and checksums at n = 1; (b) seeded small programs around the compiled shapes: int32 overflow at `+` and `*` (to double), double/int mixes, array holes and out-of-bounds reads, a changed argument shape (for `am3`: `this` or `w` with another shape, a non-array `array`), a `valueOf` on an operand. Seeds 1..200 (est.); the generator lives in `spikes/spike-c` (SPIKEC role, month 5).
- **Compare.** For each program and seed: T0 output (printed lines, completion value, uncaught error text) = Spike C output with forcing, byte for byte. For the kernels also = node v24.22.1 output.
- **Pass:** 0 mismatches over the corpus and all seeds, and every guard of every compiled function was forced at least once across the corpus (a guard never forced is an untested exit: the method fails, not the compiler). The report lists the exits taken per guard.

## 6. SP-Facts (C5)

### 6.1 Lattice and ref nodes (C5a)

Checked on every compiled graph (`spike-c structural`, from SPIKEC-1 on):
- every value carries one trust level of proven ⊑ predicted{source} ⊑ unknown (H30); PROVEN only from the H30 PROVEN list (`&`, `|`, `^`, `~`, `<<`, `>>` → int32, constants, spec invariants);
- every predicted fact has a guard (H32, JIT mode); no HINTED fact becomes PROVEN;
- the graph names a heap value only through the five ref nodes (RootRef, IntrinsicRef, ShapeRef, CellRef, UnitConst); there is no raw heap constant node (`spike_c::ir::RefNode`).

### 6.2 Position-independent deopt metadata (C5b)

The per-exit record holds the pc offset from the code start, the bytecode offset, the value locations and the exit-counter index (H29-Q1, Q2, Q5); no absolute address. Check: copy the code and its metadata to a second place in the code range, run there with forcing, and get the same exits and output. The record format and bytes per exit are reported for H29-Q1 against R14's 12.5-17.1 exits per KB.

First reading (SPIKEC-3, ed620e8, `am3`, mac a64; a report, not a Verdict): one 12 B exit stub per guard (`movz x16, guard id; movz x17, bytecode offset; b shared exit`; it clobbers only x16 and x17) and one shared exit per function, which is the "deopt not built" trap until month 5. `am3` code 3292 -> 2352 B; out-of-line exit code 1344 -> 404 B (29 stubs of 12 B = 348 B, plus a 56 B shared exit): 46.3 -> **13.9 B per exit**, **12.6 exits per KB** of code, inside R14's 12.5-17.1. This counts the exit code only; the per-exit metadata record (C5b) does not exist yet (SPIKEC-4 writes its form).

Second reading (SPIKEC-4, a8155df and e776784, `am3`, mac a64; a report): the per-exit metadata form exists (`spikes/spike-c/src/deopt_meta.rs`: pc offset, guard, bytecode offset, counter, the place and form of every live value; offsets and indexes only): 29 records, 1069 B, **36.9 B per exit** (10 values per exit, untuned; an H29-Q1 input), byte-equal for the same function compiled in two live isolates. A call-form exit stub (`bl` to the shared exit; the exit index comes from the return pc, MC-w15-14) is 4 B per stub: 14.8 B per exit with the in-code id/offset table, 6.8 B without it (the metadata records already hold the guard id and offset). The month-5 deopt entry saves x16/x17 (and x30 for the call form) before any table call or far branch.

### 6.3 PROFILED file vs live (C5c)

- **File.** `spike-c facts <kernel> --out <file>`: the live facts after a training run (n = 1), relabelled PROFILED (`spike_c::facts::to_profiled`), in the FactSnapshot encoding of `ep9_facts` (`FACT_SCHEMA_VERSION` 1). The `.ep9prof` container is H16-Q5b (A3).
- **Load.** A new process reads the file with `spike_c::facts::read_profiled`: decode, the source must be PROFILED, then `FactSnapshot::validate` against the unit (a stale file is refused; a record that does not fit is dropped and listed). The same graph builder compiles the hot set from it, before the kernel's first `run()`.
- **Kernel.** crypto (kernel 1) first.
- **Pass:** identical output to T0 and to config C; speed t(C) / t(P) ≥ 0.90 (est.) with the class S rule of §3 (pass: the whole interval ≥ 0.90). Time box: 2 weeks beyond the SP-Facts budget; after it a speed-only miss is recorded in H30 and does not block M1 exit (09-milestones.md:238-241).

## 7. Steps and owners

| Phase | Role | Step | Acceptance |
|---|---|---|---|
| M4-P0 | P0-SPIKEC | crate, harness, kernel drivers, facts input, stubs, `gate.sh structural`, this protocol | `cargo test`, SPIKEC(structural) |
| M4-P1 | SPIKEC-1 | IR (lattice, ref nodes), graph builder, guards with the trap, linear scan, a64 emit; `am3` first | `gate.sh am3`: `crypto` output equal to T0 (may be "expected fail" until SPIKEC-2, with the reason) |
| M4-P2 | SPIKEC-2 | `am3` from live facts, output equal to T0; the class S harness on the Mac (report) | SPIKEC(structural, am3) |
| M4-P3 | SPIKEC-3 | keyed facts (FACTS2); guard placement and exit stubs; first `am3` timing on the Mac (report) | Done (8c0e254, ed620e8): SPIKEC(structural, am3) PASS. Keyed facts back only the key's int32 check in FACT_SCHEMA 1 (a record with keys = {index}); CheckArray, LoadElement and StoreElement stay unbacked, because schema 1 drops the receiver, out-of-bounds and hole bits (month-6 bump, MODES-D18); a site that saw a non-index key is refused. `am3`: 11 fact + 18 unbacked guards, as before (every `am3` key is an int32 loop phi, so no key check is left). Exit stubs: one per guard plus one shared exit (§6.2). Checks are shared by (op, input, exit policy), never by value alone (MC-w14-20). Mac timing (`gate.sh time`, a report, never a reading): R = 4.139 [4.039, 4.318] and 4.124 [4.001, 4.242], both void (IQR 6.4% and 5.5% at load 8.35 and 9.1; other agents loaded the host); 0 exits, every run right |
| M4-P4 | SPIKEC-4 | `lin_solve` graph build; the per-exit PI metadata form (no runtime) | Done (469d1b6..e776784): SPIKEC(structural, am3, linsolve) PASS. `vm_layout.rs` reads `ep9_vm::layout` (each constant names its owning version); keyed sites that saw a non-index key, and stores of a non-int32 value, call RuntimeTable 8 and 9 (`get_keyed`, `set_keyed`) and record no feedback. `am3` code bytes are unchanged (2352 B). Exit metadata and the call-form stub: §6.2. `lin_solve`: 140 nodes, 22 fact + 23 unbacked guards, 80 intervals, 0 spilled; the emitter stops at the first context-slot load (no CONTEXT offsets in the export, §9) |
| month 5 | SPIKEC + VM | deopt entry (C2), tier-differential (C3), SP-Facts load (C5); the rest of the hot sets | the §4-§6 pass rules |
| month 6 | orchestrator | gate windows (C4, C5c), Verdict rows; the lazy-deopt probe (time box) | §3, §6.3 |

## 8. Verdict and reopen

- The orchestrator writes one Verdict row per condition of §0 in `docs/measurements/<date>-spike-c.md`, each with its numbers, host, CPU model and load.
- A correctness or structure failure (C1-C3, C5a, C5b, or identical output in C5c) reopens the named ADRs: H2, H14, H16, H17, H29 (Spike C); H30-H34 (SP-Facts) (09-milestones.md:252, :240).
- A speed miss of C4 follows the 09 §4 rule for a failed spike gate; a speed-only miss of C5c after the time box is recorded in H30 (09-milestones.md:241).

## 9. Inputs that do not exist yet (found while writing this)

| Need | Why | Where it lands |
|---|---|---|
| install by function id | the navier-stokes hot functions are inner closures of `FluidField`: no JS expression reaches them, and `ep9_vm::native_target` / `install_native_b` take a function value | a `spike-native` item in `crates/ep9_vm/src/native.rs` (no month-4 owner, plan v9 rule 12): the fixer or month 5; until then `spike-c compile navier-stokes` reports it |
| a per-function T0 profile in ep9 | §2.1 uses node --jitless shares; C4 depends on the real T0 share of each hot set | the §3 report computes the implied share; a direct T0 profile is not planned |
| a CI step for `gate.sh structural` | SPIKEC(structural) runs in the orchestrator's local CI (plan v9 §1.2); `.github/workflows/ci.yml` has no spike-c step | the CI role, if the orchestrator wants it on GitHub |
| a `layout` export of the `vm_layout.rs` copies (`FI_FEEDBACK`, the object, array and ELEMENTS offsets, the IC layout) | Spike C copies private VM layouts (`spikes/spike-c/src/vm_layout.rs`). The copies fail closed (a session checks them on live objects before any compile), but a layout change refuses every compile until the copy is fixed; doc 13 §5.2 asks that native code use the named `ep9_vm::native::layout` constants (SPIKEC-2 carry row) | `crates/ep9_vm/src/native.rs` `layout`: M4-P3 KEYED (decision P2-ORCH-6). Done in 8140b68 (merged after SPIKEC-3, whose base 830a788 did not have it), still behind `spike-native` until C-18 moves it (M5-P0). Spike C still uses its copies: the switch of `vm_layout.rs` to `ep9_vm::layout::*` goes to SPIKEC-4 (M4-P4; orchestrator decision P3-ORCH-9) |
| keyed load and store `RuntimeTable` entries | keyed sites keep the CheckArray, int32 key, bounds and hole guards, which end in the trap; a generic keyed path needs a runtime entry that native code can call (SPIKEC-2 carry row; in am3 these are the 6 unbacked guards: CheckArray x2, LoadElement x3, StoreElement x1) | `crates/ep9_vm/src/native.rs`: M4-P3 KEYED (P2-ORCH-6). Done in 8140b68: `get_keyed` and `set_keyed` (`layout::RT_GET_KEYED`, `RT_SET_KEYED`; they record no feedback). Spike C still refuses a non-index keyed site; using them as its generic keyed path goes to SPIKEC-4 (P3-ORCH-9). The element guards stay unbacked until the month-6 schema bump gives the keyed bits (MODES-D18) |
| CONTEXT cell slot offsets in `ep9_vm::layout` | `lin_solve` has 11 LoadContextSlot nodes; the emitter refuses the first one, because the export has no CONTEXT offsets (SPIKEC-4) | `crates/ep9_vm/src/native.rs` `layout` (no month-4 owner, plan v9 rule 12): month 5, with "install by function id" |
| a guard failure that JS cannot catch | until the deopt entry exists, a failed guard ends in the trap, which throws a catchable JS error; a JS `try` around the call could catch it (SPIKEC-2 carry row; am3 green does not depend on it) | the month-5 deopt entry (C2, §4) replaces the trap |

## Change log

| Date | Change | Row |
|---|---|---|
| 2026-10-26 | Written (M4-P0 P0-SPIKEC), before any compiler code | - |
| 2026-10-26 | Codex review round 1, before any gate run: §3 a wrong run fails the gate, not only the run (C4 row too); §2 the driver checks each file's blob id against the pin and needs the check line | - |
| 2026-10-26 | Codex review round 3, before any gate run: §2 the navier-stokes check line adds order-sensitive digests of every density and velocity cell (boundary cells too) and refuses a non-finite cell; the three sums alone kept a swap of two unequal cells. Node's n = 1 line is recomputed (node v24.22.1); C1-C3 and §3 compare the whole output with T0, and the check line is made outside the timed region | - |
| 2026-10-27 | M4-P2 integration fixer (SPIKEC-2 fixer need 2): §2.1 note (one compiled function gives crypto R ≈ 4.0 on the Mac report, because ep9 T0 spends about 88% in am3; est., loaded host); §3 names `spike-c time` / `gate.sh time` as the SPIKEC-2 class S harness; §9 gets the three SPIKEC-2 carry rows (layout export, keyed runtime entries, the catchable trap). No condition, pass rule or kernel changes | - |
| 2026-10-27 | M4-P3 integration fixer (SPIKEC-3 fixer need 2; P3-ORCH-9): §6.2 gets the first bytes-per-exit reading (13.9 B per exit, 12.6 exits per KB; R14: 12.5-17.1); the §7 SPIKEC-3 row gets its result (keyed facts back only key checks in schema 1, exit stubs and a shared exit, the void Mac timing); §9: the layout export and the keyed RuntimeTable entries are done by KEYED (8140b68), and the Spike C switch to them goes to SPIKEC-4 (P3-ORCH-9). No condition, pass rule or kernel changes | - |
| 2026-10-27 | M4-P4 integration fixer A (SPIKEC-4 fixer need 3): §2.2 the lin_solve record count after FACTS2 and KEYED; §6.2 the second reading (per-exit metadata 36.9 B per exit, the 4 B call-form stub); §7 the SPIKEC-4 row result; §9 a CONTEXT-offsets row. The layout-export and keyed-entry rows of §9 stay as written (done by KEYED; Spike C uses them since 469d1b6, §7). No condition, pass rule or kernel changes | - |
