# SP-Dispatch on x86-64, M0 week 2 (2026-10-21): the sibling-vs-match gate fails on x64

- Spike: [09-milestones](../plan/09-milestones.md) section 4, row SP-Dispatch. It validates H15 ([ADR](../adr/H15-interpreter-dispatch.md)).
- Code: [`spikes/sp-dispatch`](../../spikes/sp-dispatch/README.md); `cargo xtask disasm-check` in `tools/xtask/src/disasm.rs`.
- Result: on x86-64 the gate "sibling call ≥ 1.2x faster than match" fails, and match beats sibling on a kernel. This is the DISPATCH-GATE reopen trigger ([decisions-log §5.4](../decisions-log.md)). The cause: LLVM already makes the x86-64 match loop threaded code. The decision is in the H15 ADR and in [decisions-log §7](../decisions-log.md).

```text
                      match loop as built                     sibling calls
aarch64 (all hosts)   1 dispatch jump: a real switch loop     1 jump per handler   -> sibling 1.5-2.1x faster
x86-64 (SysV, Win64)  16 dispatch jumps: LLVM copied the      1 jump per handler   -> 0.77-1.13, by CPU model
                      dispatch into the arms (threaded)
```

## 1. CI run 1: the failure

Run [39458801246](https://github.com/Brooooooklyn/ep9/actions/runs/39458801246), commit `ea31dcb`, hosted runners, 15 rounds. ns per dispatch (median), match / sibling / become / cgoto.c:

| Kernel | linux-x64 ([job](https://github.com/Brooooooklyn/ep9/actions/runs/39458801246/job/118752542012)) | linux-arm64 ([job](https://github.com/Brooooooklyn/ep9/actions/runs/39458801246/job/118752542004)) | macos-arm64 ([job](https://github.com/Brooooooklyn/ep9/actions/runs/39458801246/job/118752541955)) |
|---|---|---|---|
| loop | 1.183 / 1.196 / 1.228 / 1.015 | 1.474 / 0.826 / 0.828 / 0.835 | 1.520 / 0.721 / 0.714 / 1.455 |
| branchy | 1.544 / 2.672 / 2.741 / 2.137 | 1.577 / 0.942 / 0.939 / 0.953 | 1.498 / 0.879 / 0.882 / 1.586 |
| call | 1.315 / 1.403 / 1.390 / 1.247 | 1.823 / 1.025 / 1.054 / 1.047 | 2.034 / 0.843 / 0.837 / 1.853 |
| match / sibling, geomean | **0.812 FAIL** (per kernel 0.989, 0.578, 0.937) | 1.745 pass | 2.054 pass |
| sibling / cgoto, geomean | 1.184 pass | 0.986 pass | 0.500 pass |

- The log printed no CPU model. Its numbers match the AMD EPYC 7763 jobs of §3 (branchy 1.544 / 2.672 / 2.137 vs 1.51 / 2.57-2.73 / 2.05-2.19).
- windows-x64 did not build cgoto.c (no `clock_gettime`). That is fixed (`now_ns()` with QueryPerformanceCounter); §5 has the Windows numbers.
- macos-arm64 sibling / cgoto = 0.500: the cgoto.c build of the runner (Apple clang) was 2x slower than locally. Not verified why; the run did not print the compiler. gate.sh prints it now.

## 2. Cause: the x86-64 match loop is already threaded

Static check, rustc 1.99.1, fat LTO, `match_loop::run` (now exported as `ep9loop__match__run`):

| Target | Indirect jumps in the match loop | Same with `-C llvm-args=-disable-tail-duplicate -disable-early-taildup -tail-dup-placement=false` | Dispatch-counting copy (`run_counting`) |
|---|---|---|---|
| x86_64-unknown-linux-gnu | 16 (`movzbl; movslq; add; jmpq *%rcx` in each arm) | 1 | 1 |
| x86_64-pc-windows-msvc | 16 (arm sizes equal SysV; 13 of 315 instructions differ, all in entry and exit) | 1 | 1 |
| x86_64-apple-darwin | 17 | 1 | 1 |
| aarch64 (Linux, macOS) | 1 (every arm branches to one `br`) | 1 | 1 |

- The counting copy adds one memory increment per dispatch. That alone stops LLVM from copying the dispatch on x86-64. So the threaded shape is fragile. `cargo xtask disasm-check --match-loop` now checks it on x86-64 (rule: dispatch jumps ≥ half of the 19 fast tail handlers; no stack access away from a call, the entry or an exit). The gate also runs the check on a build without tail duplication: it must fail for the reason "the dispatch is not copied into the arms".
- Exporting the loop symbol changes only one entry instruction (`acc` comes from the argument, not a constant 0). The arms are byte-identical (checked on SysV, Win64 and aarch64 objects).

Instructions per dispatch, cachegrind (diag run 1, x64 job [118771980068](https://github.com/Brooooooklyn/ep9/actions/runs/39464428627/job/118771980068) and arm64 job [118771979665](https://github.com/Brooooooklyn/ep9/actions/runs/39464428627/job/118771979665)):

| | loop | branchy | call |
|---|---|---|---|
| x64 match | 8.273 | 7.947 | 13.579 |
| x64 sibling | 6.727 | 6.594 | 12.947 |
| x64 cgoto.c (clang) | 6.091 | 5.774 | 11.947 |
| arm64 match | 10.045 | 9.624 | 14.632 |
| arm64 sibling | 6.182 | 5.872 | 10.895 |

x64 match runs only 1.05-1.23x the instructions of sibling; arm64 match runs 1.34-1.64x. The R06 figure "sibling 1.3-1.5x faster than match" came from Apple M only.

## 3. Sibling vs match by CPU model (hosted `ubuntu-24.04` and `ubuntu-24.04-arm`)

Diag branch `diag/dispatch-x64` (not merged): runs [39464428627](https://github.com/Brooooooklyn/ep9/actions/runs/39464428627), [39466346055](https://github.com/Brooooooklyn/ep9/actions/runs/39466346055), [39468143183](https://github.com/Brooooooklyn/ep9/actions/runs/39468143183). 15 rounds, medians. Columns: match time / sibling time per kernel and geomean (> 1: sibling faster); match / cgoto.c and sibling / cgoto.c (time, geomean). Same binaries and kernels as the gate.

| Run | Job | CPU | loop | branchy | call | match / sibling | match / cgoto (max kernel) | sibling / cgoto |
|---|---|---|---|---|---|---|---|---|
| 39464428627 | 118771980068 | AMD EPYC 7763 | 0.954 | 0.556 | 0.880 | **0.775** | 0.915 (1.076) | 1.180 |
| 39464428627 | 118771980233 | AMD EPYC 7763 | 0.984 | 0.585 | 0.934 | **0.813** | 0.936 (1.088) | 1.151 |
| 39466346055 | 118778639283 | AMD EPYC 7763 | 0.975 | 0.586 | 0.901 | **0.801** | 0.919 (1.093) | 1.147 |
| 39466346055 | 118778639454 | AMD EPYC 7763 | 0.918 | 0.574 | 0.900 | **0.780** | 0.869 (1.014) | 1.115 |
| 39466346055 | 118778639625 | AMD EPYC 7763 | 0.976 | 0.578 | 0.857 | **0.785** | 0.906 (1.164) | 1.154 |
| 39468143183 | 118784846504 | AMD EPYC 7763 | 0.941 | 0.560 | 0.869 | **0.771** | 0.902 (1.081) | 1.170 |
| 39468143183 | 118784847094 | AMD EPYC 7763 | 0.960 | 0.578 | 0.858 | **0.781** | 0.921 (1.103) | 1.180 |
| 39466346055 | 118778639808 | AMD EPYC 9V74 | 0.913 | 0.828 | 0.911 | **0.883** | 1.056 (1.163) | 1.196 |
| 39466346055 | 118778639835 | AMD EPYC 9V74 | 1.019 | 0.833 | 0.903 | **0.916** | 1.066 (1.222) | 1.164 |
| 39468143183 | 118784846975 | AMD EPYC 9V74 | 0.977 | 0.834 | 0.965 | **0.923** | 1.061 (1.176) | 1.150 |
| 39468143183 | 118784847179 | AMD EPYC 9V45 | 1.021 | 0.995 | 1.061 | **1.025** | 1.008 (1.054) | 0.983 |
| 39464428627 | 118771980329 | Intel Xeon Platinum 8573C | 1.307 | 1.095 | 1.014 | **1.132** | 1.189 (1.306) | 1.051 |
| 39464428627 | 118771979665 | Neoverse-N2 | 1.789 | 1.632 | 1.793 | **1.736** | 1.737 (1.766) | 1.000 |
| 39466346055 | 118778639691 | Neoverse-N2 | 1.791 | 1.703 | 1.804 | **1.765** | 1.605 (1.768) | 0.909 |
| 39468143183 | 118784847040 | Neoverse-N2 | 1.744 | 1.602 | 1.796 | **1.712** | 1.749 (1.769) | 1.022 |

- One runner label gave 4 x86-64 CPU models in 12 jobs. The verdict changes with the model: 0.77-0.81 (Zen 3), 0.88-0.92 (Zen 4), 1.03 (Zen 5), 1.13 (Intel).
- The ranges do not overlap on EPYC 7763 branchy. Example, job 118784847094: match 691-837 ms, sibling 1208-1421 ms.
- `branchy-pred` (branchy with predictable branches) gives 0.97-1.04 on EPYC 7763. Sibling loses only when branches mispredict: on Zen 3 each extra mispredicted branch costs sibling about 5x what it costs match. The hardware cause is not found: the hosted AMD VMs expose no PMU ("PERF unavailable").
- The x64 sibling code has no defect: handlers have no frame, 5-12 instructions and one `jmp *(%rdi,%r8,8)`; 0.6-1.0 instructions per dispatch more than clang's cgoto.c.

## 4. Probes (no DSL fix found)

Geomean match / X over the 3 kernels (> 1: X faster than the shipped x64 match loop):

| Probe X | EPYC 7763 | EPYC 9V74 | Intel 8573C | Neoverse-N2 |
|---|---|---|---|---|
| sibling (gate build) | 0.771-0.813 | 0.883-0.923 | 1.132 | 1.712-1.765 |
| `jmp reg` instead of `jmp [mem]` | 0.803-0.804 | - | 1.129 | 1.756 |
| no tail duplication (sibling) | 0.818-0.850 | - | 1.196 | 1.758 |
| JumpIfFalse with `cmov` (sibling) | 0.733-0.743 | 0.738-0.759 | - | 1.618 |
| one taken jump per handler | 0.582-0.593 | 1.003 | - | 1.456 |
| functions 64-byte aligned (sibling, `-align-all-functions=6`) | 0.822-0.899 | 1.066-1.127 | 1.307 | 1.730-1.745 |
| `-C target-cpu=znver3` (sibling) | 0.838-0.856 | 1.045-1.103 | - | - |
| one-jump match (tail duplication off) | 0.733-0.737 (sibling 1.09-1.15x faster) | - | 0.705 | 1.005 |
| cgoto.c, gcc -O2 | 0.891-0.971 | 0.867-0.896 | 1.089 | 1.766-1.773 |

- No probe makes sibling 1.2x faster than match on any AMD model. Even C computed goto is not faster than the x64 match loop on Zen 3: match time / cgoto.c (clang) time = 0.87-0.94 (§3).
- 64-byte alignment helps on Zen 4 and Intel but needs a global LLVM flag on stable Rust (it aligns every function of the binary), and it still loses on Zen 3. Not applied.

## 5. Windows x64 (`windows-2025`)

Diag run [39463155980](https://github.com/Brooooooklyn/ep9/actions/runs/39463155980) (branch `diag/win-cgoto`, deleted), 15 rounds, clang 20.1.8 for cgoto.c. The CPU model was not printed.

| Attempt | loop | branchy | call | match / sibling | match / cgoto (max kernel) | sibling / cgoto |
|---|---|---|---|---|---|---|
| [1](https://github.com/Brooooooklyn/ep9/actions/runs/39463155980/job/118767490238) | 1.251 | 1.026 | 1.035 | 1.099 | 1.071 (loop 1.384) | 0.975 |
| [2](https://github.com/Brooooooklyn/ep9/actions/runs/39463155980/job/118769727300) | 1.443 | 0.553 | 1.018 | 0.933 | 1.049 (loop 1.664) | 1.125 |

- The Win64 match loop has the same code as SysV (§2). The two new-gate runs of §7 named the Windows CPUs: AMD EPYC 7763 and AMD EPYC 9V74.
- Loop kernel on EPYC 7763, ns per dispatch, Linux (7 diag jobs) vs Windows (§7 run 1): match 1.10-1.15 vs 1.254; sibling 1.16-1.20 vs 0.978; cgoto.c 0.98-1.09 (clang 18.1.3) vs 0.802 (clang 20.1.8). Sibling and cgoto.c are 17-22% faster on Windows, match is 9-14% slower. So the per-kernel guard "match ≤ 1.5x cgoto.c" fails on loop (1.591, §7) mostly because the baseline moved. H15-Q3 in the ADR follows this up (pin the cgoto.c compiler, repeat).

## 6. Local gate after the change (provisional)

`bash spikes/sp-dispatch/gate.sh`, Apple M5 Max, 2026-10-21 02:09-02:10, load 1.3-2.2 (other agent workflows ran on the host, so it was not a quiet window). 15 rounds, PERF-NOISE verdicts:

| Gate | Value (median of paired ratios, interval) | Status |
|---|---|---|
| sibling-vs-cgoto-a64 | 0.975 [0.928, 0.981]; per kernel 0.985, 0.999, 0.944 | pass |
| sibling-vs-match-a64 | 1.489 [1.460, 1.511]; per kernel 1.720, 1.256, 1.537 | void (call IQR 5.0% > 5%; interval verdict pass) |
| dsl-3-expansions, handler-args, tail-exits-a64 / -x64-sysv / -x64-win64 | static and run checks | pass |
| match-loop-x64-sysv, match-loop-x64-win64 | 16 dispatch jumps (need ≥ 10); negative build fails as expected | pass |

The run exits 1 in gate mode (void), as the speed-gate rule says. Only a quiet-window run on the gate host can pass or fail the a64 speed gates.

## 7. CI runs of the new gate (branch `diag/h15-gate`)

The spikes job of the new `ci.yml` (only that job, byte-identical), `SP_SPEED=report`, 31 rounds. Runs [39472609604](https://github.com/Brooooooklyn/ep9/actions/runs/39472609604) (commit `309bb78`) and [39474039711](https://github.com/Brooooooklyn/ep9/actions/runs/39474039711) (commit `0002cc9`, annotation escapes). All 4 jobs passed in both runs (structural gates and the x64 negative build). Median of the paired ratios [interval], PERF-NOISE status:

| Run, job | CPU (printed) | cgoto.c compiler | Release gate (release / cgoto.c) | per kernel loop, branchy, call | sibling vs match (match / sibling) |
|---|---|---|---|---|---|
| 1, [linux-x64](https://github.com/Brooooooklyn/ep9/actions/runs/39472609604/job/118800281577) | Intel Xeon Platinum 8370C | clang 18.1.3 | match 1.128 [1.127, 1.129] **pass** | 1.249, 1.088, 1.056 | report: 1.126 (sibling faster) |
| 2, [linux-x64](https://github.com/Brooooooklyn/ep9/actions/runs/39474039711/job/118805251684) | Intel Xeon Platinum 8370C | clang 18.1.3 | match 1.128 [1.127, 1.129] **pass** | 1.250, 1.087, 1.057 | report: 1.128 |
| 1, [windows-x64](https://github.com/Brooooooklyn/ep9/actions/runs/39472609604/job/118800281500) | AMD EPYC 7763 | clang 20.1.8 | match 1.022 [1.004, 1.048]; **void** (IQR > 5%), interval verdict fail | **1.591** [1.550, 1.646], 0.692, 0.978 | report: 0.834 (match faster; loop 1.329, branchy 0.504) |
| 2, [windows-x64](https://github.com/Brooooooklyn/ep9/actions/runs/39474039711/job/118805251604) | AMD EPYC 9V74 | clang 20.1.8 | match 1.056 [1.037, 1.066]; **void**, interval verdict pass | 1.328, 0.791, 1.095 | report: 0.971 |
| 1, [linux-arm64](https://github.com/Brooooooklyn/ep9/actions/runs/39472609604/job/118800281581) | Neoverse-N2 | clang 18.1.3 | sibling 0.947 [0.913, 0.963]; void, interval pass | 0.873, 0.917, 1.018 | gate: 1.725 [1.693, 1.741]; void, interval pass |
| 2, [linux-arm64](https://github.com/Brooooooklyn/ep9/actions/runs/39474039711/job/118805251183) | Neoverse-N2 | clang 18.1.3 | sibling 1.013 [1.002, 1.027]; void, interval pass | 0.992, 1.015, 1.015 | gate: 1.711 [1.691, 1.726]; void, interval pass |
| 1, [macos-arm64](https://github.com/Brooooooklyn/ep9/actions/runs/39472609604/job/118800281157) | Apple M2 Pro (Virtual) | Apple clang 17.0.0 | sibling 0.483 [0.476, 0.490]; void, interval pass | 0.490, 0.558, 0.412 | gate: 2.091 [2.071, 2.119]; void, interval pass |
| 2, [macos-arm64](https://github.com/Brooooooklyn/ep9/actions/runs/39474039711/job/118805251642) | Apple M2 Pro (Virtual) | Apple clang 17.0.0 | sibling 0.486 [0.481, 0.489] **pass** | 0.491, 0.557, 0.420 | gate: 2.079 [2.074, 2.104] **pass** |

- The CPU line works on all 4 targets (CPUID brand string on x86-64, including Windows).
- A fifth x86-64 model: Intel Xeon Platinum 8370C. Sibling is 1.13x faster than match there, as on the Xeon 8573C (1.13). The vendor split holds: match is faster on AMD Zen 3 and Zen 4, sibling on both Intel models.
- Hosted Linux x64 was very quiet (IQR 0.1-0.3%). The hosted arm64 and Windows runners were void in most runs (IQR > 5% on a kernel). The verdict labels show it; the numbers stay in the job summary and `bench.json`.
- macos-15 runs Apple clang 17.0.0; its cgoto.c build is 2x slower than sibling (cgoto 1.52 ns per dispatch on loop). Locally Apple clang 21.0.0 and Homebrew clang 23.1.3 give the same `run` code (20 indirect branches) and the same time (314.0 vs 313.0 ms on loop). The old Apple clang is the probable cause; not verified on that runner.
- Each speed gate that did not pass gave one `::warning` annotation with the CPU, the verdict and the geomean line. Run 2 escapes `,` and `:` in the title, so the full CPU name shows.

## 8. CI rule (from 2026-10-21)

```text
spike gates job ── structural gates ── blocking (exit code): build, tests, agreement, deep loop,
                │                      tail exits, handler args, x64 match-loop shape, negatives
                └─ speed gates ─────── SP_SPEED=report, 31 rounds: never in the exit code.
                                       Each non-pass -> ::warning annotation; every gate ->
                                       job summary table; all samples -> artifact
                                       sp-dispatch-gate-<target>/bench.json
```

## 9. Open

- Hardware cause of the Zen 3/4 mispredict cost in sibling code: needs a host with a PMU (bare-metal Zen 3) and `perf stat` with `ex_ret_brn_ind_misp`.
- Coverage: two Intel server models, no Intel or AMD client CPU.
- The cgoto.c baseline compiler differs per target (clang 18.1.3 Linux, clang 20.1.8 Windows, Apple clang 17.0.0 on macos-15, Homebrew clang 23.1.3 locally). Ratios against cgoto.c depend on it (§5, §7). Pin one LLVM version for cgoto.c on every target, then repeat the Win64 EPYC 7763 run.
