# Verdicts (fixture)

The plan v10 form (V10-D37), one row for each protocol condition (M5-P4-ORCH-3), and for C4 one
row per gate kernel (protocol §0, §2; Codex r2). The tests put the fake repo's HEAD in place of
the placeholder commit `c0` x 20, which is in no repository.

| Spike | Condition | Verdict | Commit | Lowering | Host/CPU | ADRs |
|---|---|---|---|---|---|---|
| Spike C | C1 | GO | `c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0` | `jit` | m5max-macos, Apple M5 Max | H2, H14, H16, H17, H29 |
| Spike C | C2 | GO | `c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0` | `jit` | m5max-macos, Apple M5 Max | H2, H14, H16, H17, H29 |
| Spike C | C3 | GO | `c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0` | `jit` | m5max-macos, Apple M5 Max | H2, H14, H16, H17, H29 |
| Spike C | C4 crypto: R 3.4 [3.2, 3.5], 15 rounds, IQR 2% | GO | `c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0` | `jit` | m5max-macos, Apple M5 Max | H2, H14, H29-H34 |
| Spike C | C4 navier-stokes: R 3.1 [3.0, 3.2], 15 rounds, IQR 3% | GO | `c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0` | `jit` | m5max-macos, Apple M5 Max | H2, H14, H29-H34 |
| SP-Facts | C5a | GO | `c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0` | `jit, PROFILED input` | m5max-macos, Apple M5 Max | H30-H34 |
| SP-Facts | C5b | GO | `c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0` | `jit, PROFILED input` | m5max-macos, Apple M5 Max | H30-H34 |
| SP-Facts | C5c | GO, speed recorded | `c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0` | `jit, PROFILED input` | m5max-macos, Apple M5 Max | H30-H33 |
