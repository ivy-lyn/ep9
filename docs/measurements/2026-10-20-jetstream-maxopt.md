# JetStream 3: node --max-opt=2 vs node full (2026-10-20)

- Task: [09-milestones](../plan/09-milestones.md) section 8 week 1, "node --max-opt=2 vs node full on JetStream 3 JS workloads". The ratio feeds G4 and M7 exit 3.
- **M7 list (63 workloads): node --max-opt=2 scores 0.878x node full.** The 7 repetitions give 0.861-0.892.
- **M2 list (30 workloads): 0.822x.** The 7 repetitions give 0.774-0.851.
- Cross-check: d8 15.8.24 running `cli.js` natively gives 0.887 (M7) and 0.846 (M2). So the node host does not bend the list ratio.
- So the G4 floor (est.) on this machine class is ep9 T2 ≥ 0.75 × 0.878 = **0.66x node full** on the M7 list.
- **The machine was shared and busy.** The 1-minute load average was 17-37 on 18 cores during the run. Single runs of one workload vary by up to 97%. Use the medians below, not single cells. The quiet machine of 10-quality section 2 must measure this again before a gate uses it.
- **Status: provisional** (speed-gate rule, [10-quality §3](../plan/10-quality.md#3-benchmark-sets), decision PERF-NOISE). The host was not a gate host and was loaded, so these ratios never pass or fail a gate. Measure again under the rule before M7 exit 3 uses them. The M2 list also changed after this run: decision M2-JETSTREAM-LIST keeps the Proxy, Reflect and WeakRef workloads, and `expectations/jetstream-m2.txt` now has 34 workloads (2026-10-21). The M2 ratio here is for the old 30-workload list.

## Setup

| Item | Value |
|---|---|
| Date | 2026-10-20, 20:54-21:08 (+08) |
| Host | Apple M5 Max, 18 cores, 128 GiB, macOS 27.0.1, shared with other agents |
| Engine | node v24.22.0 (V8 13.6.233.17), the pinned build from `third_party/shells.toml` |
| Configs | "V8 full" = `node`; "Maglev cap" = `node --max-opt=2` (10-quality section 3) |
| Benchmark | WebKit/JetStream branch JetStream3.0 at `06785cf861ac` (`third_party/bench-jetstream.commit`) |
| Lists | `expectations/jetstream-m7.txt` (63) and `expectations/jetstream-m2.txt` (30; a subset of M7) |
| Host API | `tools/bench/jetstream/node-host.mjs`: JSC-shell API on node. The driver and each workload get a fresh global from `vm.createContext(vm.constants.DONT_CONTEXTIFY)`, with no node interceptors, like a d8 `Realm` |
| Runner | `tools/bench/jetstream/run.mjs`: one process per workload and config. 7 repetitions. In each repetition the two configs of a workload run back to back, and their order rotates |
| Iterations | JetStream defaults (`cli.js` without `--iteration-count`); resources prefetched (default) |
| Score | The JetStream workload score: geomean of the First, Worst and Average iteration scores. Higher is better |
| Ratio | Per workload: median(max-opt=2) / median(full). Per list: geomean of these ratios (= the ratio of the JetStream list scores) |
| Runs | 882, 0 errors. Median wall time per run 0.60 s; total 877 s |

## Results

| List | Workloads | full, geomean score | max-opt=2, geomean score | Ratio (medians) | Ratio per repetition, median (min-max) |
|---|---|---|---|---|---|
| M7 | 63 | 399.1 | 350.3 | **0.878** | 0.883 (0.861-0.892) |
| M2 | 30 | 800.9 | 658.6 | **0.822** | 0.818 (0.774-0.851) |

```
ratio (max-opt=2 / full) per repetition; axis starts at 0.75, one # = 0.005
             M7 list                       M2 list
rep 0  0.877 #########################       0.813 #############
rep 1  0.861 ######################          0.774 #####
rep 2  0.892 ############################    0.851 ####################
rep 3  0.867 #######################         0.817 #############
rep 4  0.883 ###########################     0.837 #################
rep 5  0.891 ############################    0.818 ##############
rep 6  0.888 ############################    0.829 ################
```

Why the ratio is near 0.9 and not near 0.54 (inference, not measured here):
- On Octane steady state, the Maglev cap reached 54% of V8 full [R06].
- A JetStream score also counts the first iteration and the worst iterations. Those run mostly before TurboFan code exists, so a Maglev cap loses little there.
- The ratio is lowest on long, hot numeric and object loops: raytrace-private-class-fields 0.34, ML 0.51, mandreel 0.52, hash-map 0.68, splay 0.68.
- Startup and code-load workloads are near 1.0: they finish before any optimizing tier matters.

## Per workload

Medians of 7 runs. "spread" = (max − min) / median over the 7 runs of one config.

| Workload | M2 list | full | max-opt=2 | ratio | ratio per rep, min-max | spread full | spread max-opt=2 |
|---|---|---|---|---|---|---|---|
| acorn-wtb |  | 89.4 | 77.8 | 0.870 | 0.863-1.198 | 50% | 50% |
| ai-astar | yes | 1023.8 | 928.9 | 0.907 | 0.555-1.013 | 60% | 77% |
| Air |  | 945.7 | 816.7 | 0.864 | 0.798-1.057 | 26% | 19% |
| async-fs |  | 481.4 | 417.8 | 0.868 | 0.735-2.031 | 64% | 18% |
| babel-minify-wtb |  | 159.5 | 154.1 | 0.966 | 0.836-0.993 | 43% | 46% |
| babel-wtb |  | 199.2 | 186.4 | 0.936 | 0.920-1.223 | 44% | 34% |
| Babylon | yes | 1518.7 | 1494.6 | 0.984 | 0.662-1.030 | 51% | 49% |
| babylon-wtb |  | 150.7 | 128.2 | 0.851 | 0.768-1.048 | 43% | 45% |
| babylonjs-scene-es6 |  | 68.8 | 67.3 | 0.979 | 0.860-0.987 | 51% | 56% |
| babylonjs-startup-es6 |  | 32.7 | 35.6 | 1.087 | 0.924-1.319 | 78% | 61% |
| Basic | yes | 1515.4 | 1240.9 | 0.819 | 0.819-1.642 | 67% | 42% |
| bigint-noble-ed25519 |  | 100.3 | 95.5 | 0.952 | 0.899-1.454 | 66% | 50% |
| Box2D | yes | 712.7 | 721.3 | 1.012 | 0.908-1.529 | 97% | 76% |
| cdjs | yes | 313.4 | 261.5 | 0.835 | 0.515-0.905 | 23% | 62% |
| chai-wtb |  | 358.6 | 334.3 | 0.932 | 0.903-1.161 | 30% | 28% |
| crypto | yes | 2341.9 | 1666.9 | 0.712 | 0.643-0.947 | 50% | 26% |
| delta-blue | yes | 2261.0 | 1651.1 | 0.730 | 0.677-0.872 | 37% | 31% |
| doxbee-async | yes | 517.3 | 417.7 | 0.808 | 0.787-0.959 | 39% | 27% |
| doxbee-promise | yes | 474.5 | 417.1 | 0.879 | 0.617-0.972 | 37% | 62% |
| earley-boyer | yes | 1189.2 | 1066.6 | 0.897 | 0.863-1.565 | 58% | 40% |
| espree-wtb |  | 128.6 | 112.1 | 0.872 | 0.787-1.379 | 54% | 32% |
| esprima-next-wtb |  | 138.3 | 140.2 | 1.014 | 0.882-1.053 | 38% | 31% |
| first-inspector-code-load | yes | 393.5 | 399.8 | 1.016 | 0.908-1.016 | 44% | 42% |
| FlightPlanner | yes | 1431.9 | 1171.3 | 0.818 | 0.641-1.078 | 88% | 66% |
| gaussian-blur |  | 865.4 | 716.4 | 0.828 | 0.721-0.889 | 52% | 48% |
| gbemu |  | 248.8 | 249.5 | 1.003 | 0.535-1.090 | 36% | 56% |
| hash-map | yes | 941.8 | 641.1 | 0.681 | 0.438-0.758 | 23% | 48% |
| js-tokens | yes | 519.8 | 530.1 | 1.020 | 0.952-1.061 | 15% | 10% |
| jsdom-d3-startup |  | 25.1 | 25.3 | 1.007 | 0.646-1.181 | 26% | 46% |
| json-parse-inspector | yes | 411.5 | 443.9 | 1.079 | 0.957-1.172 | 23% | 15% |
| json-stringify-inspector | yes | 572.7 | 574.1 | 1.003 | 0.956-1.297 | 23% | 22% |
| lazy-collections |  | 369.4 | 337.4 | 0.913 | 0.727-0.951 | 13% | 24% |
| mandreel |  | 260.5 | 135.7 | 0.521 | 0.336-1.099 | 52% | 51% |
| ML | yes | 228.8 | 116.7 | 0.510 | 0.403-0.615 | 62% | 57% |
| mobx-startup |  | 282.1 | 272.9 | 0.967 | 0.953-1.225 | 75% | 75% |
| multi-inspector-code-load | yes | 645.4 | 648.1 | 1.004 | 0.997-1.108 | 34% | 32% |
| navier-stokes | yes | 1531.1 | 1267.2 | 0.828 | 0.815-0.853 | 38% | 35% |
| octane-code-load | yes | 2000.4 | 1886.0 | 0.943 | 0.903-1.093 | 38% | 32% |
| OfflineAssembler | yes | 310.5 | 298.4 | 0.961 | 0.790-0.996 | 50% | 61% |
| pdfjs |  | 377.5 | 362.4 | 0.960 | 0.753-1.212 | 57% | 39% |
| postcss-wtb |  | 55.4 | 53.4 | 0.964 | 0.936-1.027 | 15% | 18% |
| prettier-wtb |  | 126.3 | 126.2 | 0.999 | 0.916-1.082 | 26% | 15% |
| prismjs-startup-es6 |  | 548.3 | 554.2 | 1.011 | 0.983-1.048 | 60% | 61% |
| proxy-mobx |  | 2153.9 | 2122.9 | 0.986 | 0.603-1.073 | 20% | 42% |
| proxy-vue |  | 1061.7 | 1020.9 | 0.962 | 0.886-1.003 | 24% | 29% |
| raytrace | yes | 792.7 | 609.8 | 0.769 | 0.490-0.839 | 27% | 59% |
| raytrace-private-class-fields | yes | 614.5 | 209.3 | 0.341 | 0.310-0.354 | 15% | 21% |
| raytrace-public-class-fields | yes | 781.7 | 584.2 | 0.747 | 0.733-0.769 | 15% | 13% |
| regexp-octane | yes | 999.1 | 1022.3 | 1.023 | 0.945-1.059 | 30% | 23% |
| richards | yes | 1727.8 | 1297.3 | 0.751 | 0.553-0.839 | 23% | 40% |
| source-map-wtb |  | 440.9 | 388.7 | 0.882 | 0.758-0.970 | 29% | 21% |
| splay | yes | 390.6 | 266.3 | 0.682 | 0.414-0.759 | 48% | 62% |
| stanford-crypto-aes |  | 813.4 | 802.0 | 0.986 | 0.891-1.075 | 27% | 34% |
| stanford-crypto-pbkdf2 |  | 1594.0 | 1581.2 | 0.992 | 0.943-1.017 | 10% | 12% |
| stanford-crypto-sha256 |  | 1526.1 | 1451.5 | 0.951 | 0.934-0.993 | 28% | 25% |
| Sunspider | yes | 1197.8 | 940.0 | 0.785 | 0.527-0.967 | 68% | 64% |
| sync-fs |  | 595.9 | 491.4 | 0.825 | 0.713-0.902 | 32% | 24% |
| threejs |  | 73.4 | 71.9 | 0.980 | 0.765-1.303 | 36% | 31% |
| typescript-lib |  | 9.0 | 8.6 | 0.954 | 0.510-1.477 | 52% | 67% |
| UniPoker | yes | 962.3 | 669.7 | 0.696 | 0.546-0.768 | 47% | 59% |
| validatorjs | yes | 582.1 | 599.5 | 1.030 | 0.979-1.328 | 78% | 63% |
| web-ssr |  | 307.3 | 313.5 | 1.020 | 0.981-1.350 | 78% | 65% |
| WSL |  | 4.0 | 4.1 | 1.035 | 0.905-1.377 | 60% | 51% |

## Cross-check with d8 15.8.24

Purpose: check that the node host (`node-host.mjs`) does not bend the ratio. d8 runs `cli.js` natively, with d8 `Realm`s. Same lists, same runner, 3 repetitions, 21:13-21:17 (+08), 1-minute load 12-23.

| Shell | List | Ratio max-opt=2 / full (medians) | Ratio per repetition |
|---|---|---|---|
| node 24.22 (V8 13.6), node host, 7 reps | M7 | 0.878 | 0.861-0.892 |
| d8 15.8.24, native `cli.js`, 3 reps | M7 | 0.887 | 0.881-0.891 |
| node 24.22 (V8 13.6), node host, 7 reps | M2 | 0.822 | 0.774-0.851 |
| d8 15.8.24, native `cli.js`, 3 reps | M2 | 0.846 | 0.848-0.855 |

- The list ratios agree within 0.01 (M7) and 0.03 (M2), with two different V8 versions (13.6 and 15.8).
- Single workloads differ more (Pearson r = 0.72 over the 63 ratios). The largest gaps: raytrace-private-class-fields 0.34 (node) vs 0.55 (d8), ML 0.51 vs 0.81, splay 0.68 vs 0.97. V8 15.8 has a newer Maglev, so per-workload ratios are not portable across V8 versions.
- G4 names node 24.22 as the comparator. Use the node numbers.
- Raw data: [2026-10-20-jetstream-maxopt-d8.json](2026-10-20-jetstream-maxopt-d8.json).

## How the lists were made

- `node tools/bench/jetstream/lists.mjs` writes both lists. `--check` tells if they are stale.
- M7 list: JetStream "default" workloads tagged "js", without "WorkerTests" and "internationalization" (M7 exit 3). 63 workloads.
- M2 list: the M7 list without workloads whose code uses a feature that the plan schedules after M2 (M3: TypedArray, BigInt, Proxy, WeakRef, SharedArrayBuffer/Atomics, Intl, Temporal, modules; M4: Worker; M8: WebAssembly). 30 workloads.
  - The scan lexes each workload .js file and each preloaded `dist/` bundle, so words in strings, comments and regexes do not count. A feature test also drops a workload (safe side).
  - Run-time check: all 30 ran under node with those globals deleted from the workload realm (Proxy kept, see below). A first scan that skipped the `dist/` bundles kept 3 more workloads (babylonjs-startup-es6, jsdom-d3-startup, web-ssr). They failed this check with "Float32Array/Uint8Array is not defined", so the scan now includes the bundles.
- The plan does not define the M2 list beyond "it runs" (M2 exit 7). This rule is the infra agent's proposal; the lead should confirm it.

## Harness findings that affect ep9

| Finding | Effect | Proposal |
|---|---|---|
| The harness puts `new Proxy({}, { get })` into every workload realm (JetStreamDriver.js, class Scripts) | Every JetStream 3 workload needs Proxy. The plan schedules Proxy in M3 | A minimal Proxy (get trap) before the M2 gate, or a local harness patch in `third_party/patches/bench-jetstream/` |
| With prefetch on (default), the shell driver inflates the `.z` files with a WebAssembly zlib | `node --jitless` fails ("WebAssembly is not defined"); so will ep9 before Wasm | Run with `cli.js --no-prefetch`. `scripts/sync-bench.sh` inflates the 3 `.z` files for that mode. Checked: `node --jitless` with `--no-prefetch` runs FlightPlanner. Compare both sides of a gate in the same prefetch mode |

## Reproduce

```bash
bash scripts/sync-bench.sh && bash scripts/sync-shells.sh node d8
node=third_party/shells/node-24.22.0/node-v24.22.0-darwin-arm64/bin/node
$node tools/bench/jetstream/run.mjs --list expectations/jetstream-m7.txt --reps 7 \
  --out out.json --config full=node --config maxopt2=node:--max-opt=2
```

Raw data: [2026-10-20-jetstream-maxopt.json](2026-10-20-jetstream-maxopt.json) (every run, load averages per repetition).
