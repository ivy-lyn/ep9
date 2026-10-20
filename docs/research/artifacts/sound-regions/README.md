# Sound-regions harness (R18)

- These are the scripts and sources behind [../../18-sound-regions.md](../../18-sound-regions.md) ([R18]).
- They were copied from the research scratchpad on 2026-10-21 (decision R18-7, [decisions-log §5](../../../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction)).
- Plan users: SP-RegionCoverage (A1h) and SP-RegionBound (M7) ([09-milestones §4](../../../plan/09-milestones.md#4-spikes)).
- Only sources and scripts are here. Results, CPU profiles, build output and third-party code stayed in the scratchpad. Run the scripts again to get them.

## Layout

| Here | Research path (`$SR` = the research `work/sound-regions/` folder) | What it does |
|---|---|---|
| `region-facts/` | `$SR/finder/facts/` | Rust tool on the tsrs public API. It type-checks a project and writes per-node facts as JSON lines |
| `finder/` | `$SR/finder/` | Region finder v1: rules r1-r7 over the facts (`analyze.py`), tsconfig generator (`gen_tsconfigs.js`), helpers and tables |
| `finder/testcases/` | `$SR/finder/testcases/` | Small TS cases that each rule must accept or reject |
| `hot/` | `$SR/hot/` | Hot-time mapper: maps V8 CPU profiles to finder regions (`hotmap.py`, `analyze_all.py`, `capweight.py`, `captures.cjs`, `runall.sh`) |
| `hot/drivers/<workload>/` | `$SR/hot/<workload>/drv/` | Workload drivers for awfy, datefns, hono, vscode and zod |
| `hot/probe/` | `$SR/hot/probe/` | Profiler sanity probe |
| `probes/semantics/` | `$SR/semantics/` | Breaker probes B01-B28c (`probes.mjs`, debugger, inspector, scope and weak probes) and the source scans |
| `probes/refute/` | `$SR/refute/` | Interrupt-poll probes (`Runtime.evaluate` at a poll) |
| `probes/revise/` | `$SR/revise/` | Amdahl model for the gain estimates |
| `probes/gain/` | `$SR/gain/` | V8 flag runs for the guard-removal bound |

## Path placeholders

The scripts held absolute paths of the research machine. The copy replaced them with these placeholders. Set them, or edit the line, before a run.

| Placeholder | Meaning |
|---|---|
| `$SR` | The `work/sound-regions/` folder of a run (outputs go below it) |
| `$EP9_RESEARCH_REPOS` | A folder with the checkouts listed below |
| `$EP9_RESEARCH_WORK` | A folder with the other research harnesses: `ts-type-trust/` (TypeScript npm package and `@types`, from [../../12-ts-type-soundness.md](../../12-ts-type-soundness.md)) and `tsrs-type-oracle/corpus/awfy-ts/` (typed AWFY port, from [../../13-tsrs-type-oracle.md](../../13-tsrs-type-oracle.md)) |
| `$EP9_RESEARCH` | The parent of the two folders above |

`region-facts/Cargo.toml` uses relative path dependencies into `third_party/tsrs/crates/`, like `../tsrs-type-oracle/oracle/`. It has its own empty `[workspace]`, so it does not join the ep9 workspace.

## Pins of the research run

| Input | Pin |
|---|---|
| tsrs | `f527e984` (`third_party/tsrs.commit`) |
| node (profiler, probes) | 24.22.0 |
| TypeScript | `050880ce59e30b356b686bd3144efe24f875ebc8` (2026-04-15) |
| vscode | `01bfb5147e54f6490144e94a3d9f334b8b0a5250` (2026-10-20) |
| hono | `5ca49b0720755c585c9a9835f0a479ceea36fb52` (2026-10-17) |
| zod | `afdac65e0f80c0ccf9e8909da97c411567908b57` (2026-10-19) |
| date-fns | `717ce0a807ea4c6b540d015b5c408723175b2838` (2026-09-22) |
| pixijs | `65552017d23255c1442a7d4ac7f0d49e3225fb1d` (2026-10-10) |
| preact | `74df25ef95554c258ca3a89668dc375a8393e3a6` (2026-10-20) |
| are-we-fast-yet | `74306fec151070fd07157cefeacf19e7e0bcdc89` (2026-03-22) |

## Run (outline)

1. Run `scripts/sync-tsrs.sh` from the repo root. Then run `cargo build --release` in `region-facts/`.
2. Set the placeholders. Run `node finder/gen_tsconfigs.js`, then `region-facts` per project, then `python3 -I finder/analyze.py <repo>...`.
3. Bundle each driver in `hot/drivers/` to one `.mjs` file. The bundle command was not saved; it was esbuild-style output with source maps. Re-create it at A1h.
4. Profile each bundle with `node --cpu-prof` (V8 default) and with `node --jitless --cpu-prof`, then run `hot/runall.sh`. SP-RegionCoverage uses the lower of the two shares.

## Notes

- Outputs go to `out/` and `target/` folders, which the repo `.gitignore` drops.
- Written for tsrs `f527e984` and oxc `0.155.0`, like the other artifacts. A newer pin can need small fixes.
- Not copied: CPU profiles (151 MB), finder facts and regions (139 MB, regenerable), bundled third-party code, Static Hermes outputs, downloaded papers and sources (R18 cites their URLs), and the result files that R18 quotes.
