# Splay root-cause draft (GCPAR3, M7-P3)

Status: draft note, not an ADR. Plan v12 §10 lever Q1 (GCPAR3; decisions-log M7-PLAN-D10, M7-PLAN-D11). The parity-debt rule ([../09-milestones.md](../09-milestones.md) §5, item 1) asks for a root-cause ADR only if splay is below 0.8x node --jitless at M2 exit. This note names the missing mechanism in that form, so that the ADR (if one is needed) starts from data. All numbers are reports, not gate readings (a loaded Mac, or a box that is not a gate host).

## 1. Readings

| Reading | Tree | splay vs node --jitless | Note |
|---|---|---|---|
| GW9a (Mac M5 Max, wall) | before GCPAR1 and GCPAR2 | 0.710 [0.693-0.720] jit | the last Mac wall reading; GW9b has not run yet |
| Path B mid (x86-64 box, 2 CPUs, wall) | `m7-p2` | 1.122 [1.099-1.163] jit, 1.131 jitless | 1 GC helper; provisional (IQR > 5%) |
| Path B mid, `EP9_GC_HELPERS=0` | `m7-p2` | 0.952 [0.917-0.981] jit | the sequential mark and the lazy sweep |

The x86 box shows that the S2a helpers carry splay above 1.0x there: with 0 helpers splay is 17-23% slower. The Mac gate (M2 exit item 6, no debt valve) is a wall reading with the default helper count.

## 2. What splay does

The setup builds a tree of 8,000 nodes. Each node holds a payload tree of depth 5: 31 inner objects and 32 leaves `{array: [0, ..., 9], string: "String for key ... in leaf node"}`. The setup allocates about 74 MB (derived: 8,000 of the 24,000 payloads of the 222.6 MB that splay allocates, BENCH10 `--gc-stats`), and all of it stays live. Then 200 runs of 80 changes each insert a new node and remove an old one, so the live set stays at about 72 MB (the last major GC marks 72.3 MB) while about 148 MB more is allocated. A young cell is mostly still live at the next GC: each minor GC keeps 96-99% of the young bytes (the measure build of GCPAR2 and GCPAR3: `EP9_GC_TRACE`, one line per GC). This is the case where a generational GC gains little: the young cells survive, and each major GC marks the whole old set again.

## 3. Where splay time goes at `m7-p2` (before GCPAR3)

Mac, samply, `EP9_GC_HELPERS=0`, 3,180 samples of the isolate thread (BENCH10 form):

| Part | Share | What |
|---|---|---|
| GC mark | 22.4% inclusive | `Tracer::drain` 8.3%, `visit_slots` 6.6%, `edge` 6.0% self; 7 GCs (4 minor, 3 major); the last major GC marks 1.55 M cells (72 MB) |
| Sweep | 2.7% | lazy page sweep on allocation |
| Cell fill | 8.8% self | `memset_pattern16`: the per-cell `undefined` fill of `init_cell`, one call per cell on macOS (under `nested_object` 3.3%, `string::concat` 2.8%) |
| Object literals | 4.7% self | `nested_object`, plus about 1.5% in shape lookups per nested array (`initial_shape`, `with_elements_kind`, `root_shape_for_kind`) |
| Named loads and stores | about 18% self | `GetNamedProperty` 13.0%, `SetNamedProperty` 4.9% (the splay tree code) |
| Number to string | 4.1% inclusive | `String(key)` formats through `core::fmt` (`numconv.rs`; BENCH10 rank 8) |
| String concat | 10.2% inclusive | the leaf strings; mostly its allocation and GC |

## 4. What GCPAR3 changed

1. **The probe back-off** (`Inner::after_gc`). After a major GC the GCPAR2 policy ran one small probe minor GC to read the survival again. On splay each probe marked young cells that the next major GC marked again. Now each probe that finds high survival again skips one more probe (at most 3 in a row, `MAX_PROBE_SKIPS`); a probe that finds low survival ends the back-off. A skipped probe moves no major GC. Splay: 7 GCs (4 minor) become 5 (2 minor).
2. **The allocation fast path** (`Inner::fill_tlab`, `Heap::init_tlab_cell`). The main TLAB fills its hole with `undefined` 4 KiB ahead of the cursor, and its limit is the end of the filled part; a TLAB cell then writes only word0 and its VO bit. This removes the per-cell `memset_pattern16` call on macOS (one fill per 4 KiB step instead).

Results (a64 instructions, `ep9-bench --counter time`, 5 runs + 1 warm-up, 2 rounds, `EP9_HASH_SEED=1`, base `m7-p2`): splay -7.5% with `EP9_GC_HELPERS=0` and with the default helpers; raytrace -4.6%; earley-boyer -1.7%; no BENCH-SET row worse than +0.03%. The GC share of splay (the measure build, 0 helpers, 3 runs, loaded Mac) goes from 22.5-24.3% to 20.6-21.1%. Wall (report, loaded Mac, 10 runs): splay -5.1% with 0 helpers, -5.8% with the default helpers. Peak RSS: +0.5 MiB (170.1 to 170.6 MiB).

## 5. The missing mechanism

**Concurrent marking of the old generation (stage S2b).** After GCPAR3 the largest part of splay time is still the major GC mark: each major GC marks every live cell of the 72 MB set while the mutator waits. S2a (M7-P1, M7-P2) marks in parallel on helper threads, but the world stays stopped; with 0 helpers (the instruction floors) it is the sequential mark. node --jitless (V8) marks its old generation concurrently on background threads, so the main thread pays only the write barrier and a short final pause, and its young generation is a copying scavenger whose cost follows the live young bytes. ep9 has the S2b design (retreating-wavefront marking on helper threads, [../03-gc.md](../03-gc.md) §4), built in M5. That is the mechanism that takes the mark off the mutator; no M2 lever replaces it.

Smaller parts that are not GC (each a lever candidate, not a root cause):

- Number to string without `core::fmt` (BENCH10 rank 8; about 4% of splay at `m7-p2`).
- A cached root array shape for the arrays of a nested object literal: each `nested_object` hit looks the shape up again (about 1.5%; the cache needs a GC root, plan rule 8).

## 6. Candidates measured or read, not built

| Candidate | Why not now |
|---|---|
| A larger heap growth factor while the live set grows fast (the setup) | It would save most of the 2 setup major GCs (21 MB and 52 MB of mark; est. -4% of splay time), but it raises the peak heap, and splay is the kernel nearest to its G7 memory cap (0.78x of the 1.2x node cap). A memory decision for the gate host, not a GC lever |
| Per-site pretenuring | V8 removed allocation-site pretenuring (21e810452a, 2026-10-27); the nursery policy stays global, so compiled allocation sites bake no tenure fact (MODES-CHECK-w32 item 9) |
| Mark-loop prefetch | Slower on the M5 Max (GCPAR2, [../03-gc.md](../03-gc.md) S2a part 2) |

## 7. Next steps

- GW9b and GW9c read splay on the Mac with both builds and both helper modes. If splay is below 0.8x at M2 exit, this note becomes the root-cause ADR (09 §5), with S2b as the named mechanism and its M5 clear date.
