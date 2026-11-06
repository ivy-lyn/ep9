# m2-exit fixtures

Inputs of the `cargo xtask m2-exit` unit tests (`tools/xtask/src/m2_exit.rs`, plan v9 rule 24:
no live file of the tree is read). The tests copy them into a scratch git repository and replace
the commit placeholder with its HEAD.

| File | Producer | Form |
|---|---|---|
| `gate-pass.json`, `gate-mixed.json`, `gate-linux.json` | `tools/bench/cross/m2gate.mjs` (written by `EP9_BLESS=1 node --test tools/bench/cross/m2gate.test.mjs`) | ep9-m2gate/v1 |
| `mem.json` | `tools/bench/cross/mem.mjs` (BENCH10; this fixture is the form m2-exit reads) | ep9-mem/v1 |
| `fastdiff.json` | the FASTDIFF test (CALLHOST; this fixture is the form m2-exit reads) | ep9-fastdiff/v1 |
| `jetstream.json` | `tools/bench/jetstream/run.mjs` (its JSON plus the `ep9` block that m2-exit reads) | JetStream run |
| `exec.json` | `ep9-test262 exec --json` (a cut CI artifact) | ep9-test262 exec |
| `fuzz-fuzzilli.json` | the Fuzzilli batch | ep9-fuzz/v1 |
| `p-gates.md` | `docs/measurements/<date>-p-gates.md` | a table with Gate and Result columns |
