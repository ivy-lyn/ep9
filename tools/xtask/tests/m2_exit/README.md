# m2-exit fixtures

Inputs of the `cargo xtask m2-exit` unit tests (`tools/xtask/src/m2_exit.rs`, plan v9 rule 24:
no live file of the tree is read). The tests copy them into a scratch git repository and replace
the commit placeholder with its HEAD.

| File | Producer | Form |
|---|---|---|
| `gate-pass.json`, `gate-mixed.json`, `gate-linux.json`, `gate-win.json`, `gate-win-linux.json` (the Windows x64 and Linux x64 gate-host pair) | `tools/bench/cross/m2gate.mjs` (written by `EP9_BLESS=1 node --test tools/bench/cross/m2gate.test.mjs`; each number that is not an integer at 10 significant digits, so the bytes are the same on every host) | ep9-m2gate/v1 |
| `mem.json` | `tools/bench/cross/mem.mjs` (BENCH10; its JSON cut to the fields that m2-exit reads, no `samples`) | ep9-mem/v1 |
| `fastdiff.json` | the FASTDIFF test (CALLHOST; `crates/ep9_builtins/tests/fastpath_diff.rs`, its JSON cut to the fields that m2-exit reads) | ep9-fastdiff/v1 |
| `jetstream.json` | `tools/bench/jetstream/run.mjs` (its JSON with the `ep9` block and the config `kind` and `version` that m2-exit reads) | JetStream run |
| `exec.json` | `ep9-test262 exec --json` (a cut CI artifact) | ep9-test262 exec |
| `fuzz-fuzzilli.json` | the Fuzzilli batch | ep9-fuzz/v1 |
| `p-gates.md` | `docs/measurements/<date>-p-gates.md` | a table with Gate and Result columns |
| `pathb-below.json`, `pathb-above.json`, `pathb-none.json` (one per host-cell form of V13-D37: `FAIL-SIDE (no gate host): AGREE below`, `MARGINAL (no gate host): AGREE above`, `MARGINAL: NO AGREEMENT`) | the Path B compare of the 2 box runs (`pathb-below.json` is the month-7 Path B compare of 2026-11-11, cut to the run checks and the `item <n>:` cells, with `platform` added; in `pathb-above.json` the TS cells are made-up readings above 1.0; in `pathb-none.json` also box 2 of the jit v8-v7 cell straddles 1.0) | ep9-pathb-compare/v1 |
| `pathb-win.json` (the win32-x64 form of plan v14 §4.1 with the QuickJS runs of plan v15 V15-D17: items 1, 2 and 6 above, made-up TS cells below; its `legs` are 4 windows legs, w2 and w3 of AMD EPYC 7763, w1 and w4 of Intel Xeon Platinum 8370C) | `.github/scripts/exit-inputs.mjs pathb` (written by `EP9_BLESS=1 node --test .github/scripts/exit-inputs.test.mjs` from made-up m2gate readings of the 4 windows legs) | ep9-pathb-compare/v1 |
| `pathb-gha-linux.json`, `pathb-gha-linux-fast.json`, `pathb-gha-linux-other.json` (the Linux x64 GitHub legs of V15-D16, l1 and l2: of the windows pair's CPU model with values a little above the Windows ones, so the cross-host intervals are above 0.9; of the same model with values about 1.3x the Windows ones, below 0.9; of another model, AMD EPYC 9V74, so no same-model pair) | `.github/scripts/exit-inputs.mjs pathb` (written by the same blessed test) | ep9-pathb-compare/v1 |
| `pathb-gha-linux-unread.json` (l1 of `pathb-gha-linux.json` and l2 not read: a read error, so l2 is in `runners` only; the same-model pairs are not known, NO AGREEMENT) | `.github/scripts/exit-inputs.mjs pathb` (written by the same blessed test) | ep9-pathb-compare/v1 |
| `mem-win.json` (a Windows MEMREAD run; its load guard reads the typeperf source; the test makes the loadavg form, "no load guard", from it) | `tools/bench/cross/mem.mjs` on Windows (the PeakWorkingSetSize tool; `mem.json` with the Windows host block) | ep9-mem/v1 |
| `mem-linux-arm64.json` (the linux-a64 MEMREAD run of the exit-inputs job: no d8 rows, jsvu has no linux-arm64 V8; plan v15 V15-D18) | `tools/bench/cross/mem.mjs` on a GitHub linux-arm64 runner (`mem.json` with that host block, and no d8-jitless engine or rows) | ep9-mem/v1 |
| `p-gates-exit-form.md` | `docs/measurements/2026-11-16-p-gates-exit-form.md` (the exit form: the first table has the Gate and Result columns) | a table with Gate and Result columns |
