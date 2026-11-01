# Verdicts (fixture)

The plan v10 form (V10-D37). The tests put the fake repo's HEAD in place of the placeholder commit
`c0` x 20, which is in no repository.

| Spike | Condition | Verdict | Commit | Lowering | Host/CPU | ADRs |
|---|---|---|---|---|---|---|
| Spike C | C4 | GO | `c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0` | `jit` | m5max-macos, Apple M5 Max | H2, H14, H29-H34 |
| SP-Facts | C5c | GO, speed recorded | `c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0` | `jit, PROFILED input` | m5max-macos, Apple M5 Max | H30-H33 |
