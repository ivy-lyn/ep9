# ep9 plan: execution modes and the room for later AOT and hybrid work

- Status: Decided design (orchestrator, 2026-10-26/27); adoption in progress: see §7 and §8.
- Cites at 10b21eb, the m4-p5 tag (moved from 4941d6a, the m4-p4 tag, by the M4-P5 DOCS writer, 2026-10-28, MODES-CHECK-w17 MC-w17-1, after `cargo xtask adr-lint --base m0-exit` at 10b21eb gave 408 short cites, 0 findings against 4941d6a; 4941d6a was checked by the M4-P5 modes fixer, 2026-10-28, MODES-CHECK-w16 MC-w16-2, and by the M4-P5 integration fixer, item 30; before that the M4-P4 integration fixer A refreshed them at the M4-P4 merge head, 2026-10-27, and 5eaf080 and 035ff95 moved them at the M4-P3 merge head; MODES-D9). Every `file:line` names the line there, except in section 12, which records the critique check of the first version at fc6ac78, and in the history items of section 13, which name the lines of their own time. Section 13 says how the cites were checked (the M1 check at 281e628, item 22 for the M4-P2 refresh, items 27 and 28 for M4-P3 and M4-P4, items 29 and 30 for the m4-p4 tag).
- Cite rules from month 5 (plan v10 rules 30 and 31; plan v10 is kept outside the repo, as plan v9 is; MODES-CHECK-w17 MC-w17-1, MC-w17-8). (1) **`Cites at` bump.** The first commit of each phase (docs only) moves the `Cites at` line above to the merge head of the previous phase. It names that head by its sha, because a phase may start before the previous tag exists; the tag name is added when it exists. `cargo xtask adr-lint` reads the sha right after "Cites at " (`tools/xtask/src/exit/cites.rs:10-13`) and compares each short cite between that base and HEAD, so a cite that moved before the base is not seen: the bump keeps the base one phase old at most. (2) **Move cites with an ADR edit.** A commit that edits a cited ADR, or the file of another short form below (`golden`, `SCP`), moves the doc 13 short cites of that file in the same commit. The role that makes the edit owns `docs/plan/13-execution-modes.md :: short cites of <names>` for those names (plan v10 rule 1, check rule 12).
- Origin: the execution-modes design that the user asked for on 2026-10-26 ("proper architecture around the AOT/JIT/Hybrid modes" that keeps room to optimize AOT and hybrid later). It went through an adversarial critique (section 12) and the first weekly check MODES-CHECK-w13 (plan v9 rule 29; window d7cc797..a99838f; section 13). MODES-D1..D19 are the orchestrator decisions on it (MODES-D15, the "ran" bit, and MODES-D16 came with M4-P2; MODES-D17 and D18, the LoadKeyed "ran" bit and its schema-bump rule, with M4-P3; MODES-D19, `ics_only` keeps every object-model input, with M4-P4, DL:3248); their rows and the MODES-CHECK-w13 row go into decisions-log §14.3 at the m4-p1 tag.
- Short forms: P02 = `docs/plan/02-architecture.md`, P03 = `docs/plan/03-gc.md`, P04 = `docs/plan/04-execution.md`, P05 = `docs/plan/05-aot.md`, P08 = `docs/plan/08-repo.md`, M09 = `docs/plan/09-milestones.md`, AR = `docs/adr/README.md`, DL = `docs/decisions-log.md`, SCP = `docs/measurements/2026-10-27-spike-c-protocol.md`, Hnn = `docs/adr/Hnn-*.md`, golden = `crates/ep9_vm/tests/golden/h20-core.txt`. Plan v9 (the month-4 plan) is kept outside the repo (DL:2743); this file cites it as "plan v9 §x" or "plan v9 rule n". MC-w13-n = finding n of MODES-CHECK-w13; On = its outside-evidence item n.
- "(est.)" marks an estimate. "Reserved" means: named now, refused by code today, built only when its gate passes.

---

## 0. Summary

The plan already has the right shape. ep9 is "one engine with three deployment modes" that "share one optimizer" (P02:5). Modes "differ only in fact sources ... and in guard policy" (P02:6). This document keeps that shape. It adds no new compiler, no new IR, no new H row and no new plan role.

The risk is timing, not shape. H2, H14, H16 and H29-H34 freeze at M1 exit (AR:37). After a freeze, a change where "a check now accepts what it rejected" is a rewrite (AR:76), and G8 counts it (AR:80; P02:26). The gates before that freeze test only the JIT-mode row (SCP:28, SCP:30). So the room for later AOT and hybrid work must be written into the contracts in months 4-6.

The design has five structures:

1. **Mode = three layers plus a process permit.** Build capability (Cargo `jit`, `aot`), a process codegen permit, isolate policy (`ExecPolicy` with three presets, passed through a new entry point, never through `IsolateConfig`) and function state (code set + dispatch slot).
2. **Code set + `select()`.** Each function keeps a small list of code objects. The dispatch slot stays the one hot read. It is a cache of `select(code set, policy, session)`. One rule serves unit load, tier-up, deopt, PIN, fuse pops and the debugger.
3. **One fact flow.** FactSnapshot stays strict and canonical (layer L1). Counts, function records and digests live in side tables of their container (layer L2). Facts travel beside the CompiledUnit, never inside it. VERIFIED facts get their own region table.
4. **Identity ladder.** Each check drops the smallest thing that fails: unit, then code object, then profile function, then fact record.
5. **Room = append-only parts.** Open value ranges (kind bytes, lowering classes, relocation kinds, record tags, fuse descriptors, cell kind ids, IC handler kinds) become append-only Freeze-table parts with no close point, as H20 part 2 already is (H20:7, H20:89). Gated content freezes at the gate that **ends** its build. Hybrid-only rows freeze at K-A4P.

```
                      +--------------------- ONE OPTIMIZER (ep9_baseline, ep9_opt) ---------------------+
 FACT SOURCES         |                                                                                  |
 PROVEN   analysis ---+                                                                                  |
 RUNTIME  FeedbackVec-+--> ep9_facts::merge --> graph builder ------------> passes ----> ep9_masm --------+
 PROFILED .ep9prof ---+    (H30, one fn)        [B2: Guard.policy and       (read Guard.policy   |
 HINTED   .ep9h ------+    [B1: which sources]   fuse form set here from     as data; no mode    |
 VERIFIED region table (reserved, own format)    GuardTable + lowering]      branch)             |
                                                                [B3: finalizer]   finalize_jit -----+--> CodeObject (JIT memory)
                                                                                  finalize_unit ----+--> unit function table (.ep9u/.o)
                                                                                                           |
 RUNTIME (ep9_vm)                                                                                          v
   FunctionInfo.slot {kind, entry} <== select(code set, ExecPolicy, session) <== code set <== load | tier-up
        ^ one hot read per call                 |                                   ^
        |                                       +-- deopt, PIN, fuse pop, debugger attach/breakpoint
   ExecPolicy presets:  JIT     = Opt > Baseline > Interp                          (lowering jit only)
                        jitless = AotOpt > AotBaseline > GenInterp > Interp        (lowering jitless only)
                        hybrid  = Opt > Baseline > AotOpt > AotBaseline > Interp   (lowering jit, hybrid, jitless)
 mode branches exist in 3 places only: B1 fact selection, B2 guard creation, B3 finalizer
```

What changes now: about 20 contract and plan text items before M1a and M1 exit (section 7.1). They go as inputs to roles that plan v9 already has: ADR-A0 in M4-P2, FACTS2 in M4-P2, the month-5 fuse registry v0, the month-5/6 Spike C fills and the FREEZE-M1 batches (section 8). Six container items wait for A2 start, before any unit exists (section 7.2).

The first weekly check (MODES-CHECK-w13, plan v9 rule 29) found no broken mode rule in the code. `Mode` has one value, `Jit`, and only `Mode::exit_policy` reads it (`spikes/spike-c/src/ir/mod.rs:360-371`). M4-P1 built T0 type feedback. That added four rules and one ExecPolicy field (section 13):

- Jitless isolates do not record type feedback by default: `type feedback: full | ics_only` (MODES-D11, section 1.2).
- An empty BinaryOp or CompareOp slot means "int32 or never ran" (MODES-D10, F-R9). The month-6 schema bump adds a per-site "ran" bit, written only inside the sampled window (MODES-D15, from the FACTS2 measure).
- Each exported count names its unit (F-R7). A Call key is never written out (F-R8).
- The counter unit and the counter rule are written in M4-P2; the numbers wait for the A4 gate (MODES-D12).

---

## 1. The mode model

### 1.1 Three layers and one permit, fixed at four times

| Layer | Fixed when | Decides | Home | Today |
|---|---|---|---|---|
| L-A build capability | link time | which code **producers** exist | Cargo features `jit` (runtime codegen) and `aot` (unit emission) (P02:184; P08:115-118) | `jit` forwards to `ep9_platform` (`crates/ep9_vm/Cargo.toml:25`). Since M5-P0 (C-23) `ep9_vm` reads it only to pick its presets: the `jit` preset needs it (`crates/ep9_vm/src/exec.rs:59-61`) and it makes `jit` the default preset (`crates/ep9_vm/src/exec.rs:122-128`); no code producer exists yet |
| L-P process permit (new, F13) | before the first isolate | which producers may **run** in this process; bounded by L-A | one process-wide setting; workers inherit it | none. One 128 MiB code range serves every isolate (H07:52; H22:40), so W^X is a process property |
| L-B isolate policy | isolate creation | which code kinds may be **installed**, their rank, accepted lowering classes, thresholds, run-time fact inputs, load policy; bounded by L-P | new `ExecOptions` (opaque) passed to a new entry point `Isolate::new_with(IsolateConfig, ExecOptions)` (proposal, section 1.2) | `IsolateConfig` has 8 public fields (`crates/ep9_vm/src/isolate.rs:52-74`), among them `feedback_threshold` and `compiler: Option<&'static CompilerHooks>`. It is a core `embed:` line of the Frozen H20 core (golden:467; `crates/ep9_vm/tests/h20_core.rs:33-34`; H20:88). `--jitless` and `--unit` are named in `crates/ep9_cli/src/main.rs:2` but not parsed. Since M5-P0 (P0-VM5, C-23) `Isolate::new_with(IsolateConfig, ExecOptions)` exists (`crates/ep9_vm/src/isolate.rs:277`) with the opaque `ExecOptions` of `crates/ep9_vm/src/exec.rs` (presets `jit` and `jitless`, the `feedback` setter); its golden lines are `embed (not core)` (golden:341; orchestrator decision M5-P0-ORCH-1: FREEZE-M1 decides on the core). M5-P0 stores the options and nothing reads them yet (PROFILE6, M5-P3) |
| L-C function state | run time | which code a function **has** and **runs** | code set + dispatch slot (section 3) | one slot `{kind, entry}` per FunctionInfo (H14:69; `crates/ep9_vm/src/function_info.rs:58`) |

Rules:

| ID | Rule | Check (when built) |
|---|---|---|
| M-R1 | No crate below the compilers reads a mode name. `ep9_vm` reads `ExecPolicy` fields and slot data only | dep-guards and grep row at A2 |
| M-R2 | Without `jit`, the permit and `produce` are empty. A jitless binary never maps W^X memory (P02:133) | feature-matrix test (H27) |
| M-R3 | A JIT build can run any preset, the jitless preset included. This is an extra test only. The G9 oracle stays the build **without** `jit` (P02:27; P02:184) | test262 run with the jitless preset on the JIT build, beside the G9 run |
| M-R4 | A code object runs only if its lowering class is in `ExecPolicy.accept` (section 1.4) | loader mutation test (A2 gate style, P05:284) |
| M-R5 | The slot never holds a code object whose state is not Active (section 3.3) | select() model tests and the mode x tier differential (P04:324) |
| M-R6 | A preset that the build or the permit cannot run (for example hybrid on a jitless build) is an error from `Isolate::new_with`. Never a silent downgrade. It reuses `IsolateError::Config(&'static str)` (`crates/ep9_vm/src/isolate.rs:99-101`), because a new variant would change a Frozen core type (H20:88) | unit test per impossible preset |
| M-R7 | A function keeps its bytecode while any of its code objects is Active or has a frame on the stack. Bytecode flushing (H04-Q3, H04:80) skips such functions. Every deopt and the BYTECODE outcome need the bytecode (H35:111) | flush test with a live AOT frame (L2 flushing) |
| M-R8 | A worker isolate inherits the process permit. A worker may pick any preset inside it | worker test (M6 runtime) |
| M-R9 | No object-model decision reads `ExecPolicy`, `ExecOptions` or the code set. Shapes, transitions, element kinds, slack and field representations come out the same under every preset and every type-feedback policy. `ics_only` keeps every object-model input, allocation-site element kinds included (MODES-D19, C-27). Why: Bun's AOT changed how objects get shapes when compiled code exists (MODES-CHECK-w16 outside item O2) | the w16 O2 test: equal shape and element-kind counts under the `full` and `ics_only` feedback of the `jitless` preset (month 5: ELEM2 writes it, PROFILE6 runs it again once `ics_only` takes effect; a JIT-vs-jitless preset run when a build can make both, A2); the plan v10 rule-40 grep: no code line of the object-model files names `ExecPolicy`, `ExecOptions` or `ics_only` |

### 1.2 ExecOptions, ExecPolicy and the three presets (proposal)

Why not `IsolateConfig`: it is a core line of the Frozen H20 core (golden:467). "A changed or removed core line is a change to a Frozen part; a new line is an addition" (golden:3-5). A new field is a changed line, so its kind is rewrite (H20:88). A new entry point and a new type are new lines, so they are part-2 additions, a fill (H20:89; test `a_new_isolate_method_is_a_new_line`, H20:131).

```
 Isolate::new_with(config: IsolateConfig, exec: ExecOptions) -> Result<Isolate, IsolateError>   (new line)
 ExecOptions  opaque: no public fields; built by preset constructors and setters (est. shape)
   The golden prints a type line with its public fields only (crates/ep9_vm/tests/h20_core.rs:29-31), so a new setter later
   is one more new line (an addition). No field of ExecOptions is ever frozen.
   holds: ExecPolicy (below) + TierHooks (section 2.1, S-2)

 ExecPolicy (owned by ep9_vm; filled by ep9_cli or the embedder through ExecOptions)
   install   allowed code kinds, in rank order
   accept    accepted lowering classes, in rank order (section 1.4)
   produce   runtime producers that may run, bounded by the permit (L-P):
             T1, T2, T3, regexp JIT (RX2, under `jit`, H27:9, H27:29), run-time IC stubs, Wasm (later)
   tiering   thresholds and budgets, all "(est.)", tuned in A4 (P04:102-106; P05:328)
   facts     unit facts read at run time (PROFILED, HINTED), at lower precedence than RUNTIME
   feedback  type feedback: full | ics_only (MODES-D11). full: T0 runs the `profile.rs` record hooks.
             ics_only: T0 keeps its ICs and runs no record hook (a second T0 handler table, below);
             it keeps every object-model input, allocation-site element kinds included (MODES-D19, M-R9)
   load      policy auto | strict (strict = `--unit-strict`, P05:282; H35:90)
             forms  {o, ep9u}: which unit forms may load. iOS takes `.o` only (P08:117)

 preset    install rank                                     accept                 produce             unit facts at run time                     type feedback
 JIT       Opt > Baseline > Interp                          jit                    all producers       none in v1 (warm start reserved, F17)      full
 jitless   AotOpt > AotBaseline > GenInterp > Interp        jitless                none (also no RX2)  n/a (baked at build)                       ics_only (full in a training run, `--profile-out`)
 hybrid    Opt > Baseline > AotOpt > AotBaseline > Interp   jit, hybrid, jitless   all producers       PROFILED + HINTED, lower precedence (H33:81) full
```

- The presets are the three plan modes (P02:126-135). A new mode later (sound regions, a closed-world bundle) is a new preset or a new guard-table row. It is not a new branch in a compiler.
- The jitless preset empties every producer, regexp JIT included. V8's `--jitless` also turns off regexp JIT and Wasm (the prior-art survey of the design work, kept outside the repo).
- The hybrid rank puts Baseline above AotOpt on purpose. A re-tiered function runs instrumented T1 for a while to collect fresh feedback. That is the H33 path "hot -> instrumented T1 -> T2" (H33:29). T1 already writes the full FeedbackVector (P05:336), so "instrumented T1" is plain T1. It needs no new kind.
- **Accept lists (MODES-D13).** `accept` is one filter for every code object: unit code and runtime JIT code alike (`select()`, section 3.2). Code that T1, T2 or the hybrid re-tier makes has lowering `jit` (section 1.4). So JIT accepts {`jit`}, hybrid accepts {`jit`, `hybrid`, `jitless`} and jitless accepts {`jitless`}. A preset that runs runtime producers must list `jit`, or `select()` would never pick the code its own tier-up makes.
- The shape of `new_with` and `ExecOptions` is decided (MODES-D7, C-23). The code and its H20 part-2 row landed in M5-P0 with the `Cx` change (P0-VM5 3a74cfa; the H20 Change-log row of 2026-10-28), as an addition; the callers are `Isolate::new` (the build's default options) and the test262 runner's `--feedback` option (GATES5).
- **Type feedback (MODES-D11, MC-w13-10).** In the jitless preset T0 is the top tier, and nothing reads the T0 records except a training run. The records cost a64 navier-stokes +9.39%, richards +6.05% and crypto +1.30% (the W-PROFILE-1b share). So the jitless preset defaults to `ics_only`. A training run (`--profile-out`) and the JIT and hybrid presets use `full`. The W-PROFILE-1b reason "hot code leaves T0 once T1 exists" holds for JIT and hybrid only (MC-w13-19).
  - Build: a second T0 handler table with no record hook, chosen per `Cx`. Each `Cx` has its own handler table (`crates/ep9_vm/src/interp/mod.rs:135`), so the choice costs 0 instructions in either table. It is built in month 5 only after a SIZE-REPORT measure of the extra handler copy (the record points are listed in `crates/ep9_vm/src/profile.rs:47-69`; the size is not measured yet, and it matters for jitless and iOS).
  - Fallback if the size is too high: one flag test in each hook (option C), only if its JIT-mode cost is ≤ +0.5% on the gate set.
  - Not an off switch: `feedback_threshold = u32::MAX` makes no feedback vector (`crates/ep9_vm/src/interp/mod.rs:147-149`), and the ICs live in the vector (`crates/ep9_vm/src/feedback.rs:174-179`), so it turns off every IC too.
- **RegExp producer (MC-w13-16).** The RegExp backend is chosen at build time today: a trait of static functions with an associated `Program` type (`crates/ep9_regexp/src/lib.rs:166-184`). RX2 is a JIT producer, so it must be chosen per isolate from `produce` (M-R3). The trait does not block this. One line goes into the H27 / RX role text before RX2.

### 1.3 Mode x code-kind matrix (extends P02:141-150)

| Code kind (slot byte) | Producer | JIT | jitless | hybrid | Guard failure | Writes |
|---|---|---|---|---|---|---|
| Interp (0) | bytecode compiler, any time (H5) | start, deopt target | cold code, eval, fallback, deopt target | eval, cold, fallback, deopt target | n/a | full FeedbackVector; ICs only under `ics_only` (jitless default, 1.2) |
| Baseline (1) | `ep9_baseline`, run time | yes (M5) | no | re-tier tier (= instrumented T1) | n/a (data ICs) | full FeedbackVector |
| Opt (2) | `ep9_opt`, run time | yes (M7) | no | re-tier target | DEOPT + exit counter (H32:28) | exit counters |
| AotBaseline (3) | `ep9_baseline` via `ep9c` | no | all reachable fns if K-A3 (P02:147) | optional | n/a (data ICs) | full schema via IC cells (H33:28) |
| AotOpt (4) | `ep9_opt` via `ep9c` | no | PGO hot set if K-A4J (P02:148) | PGO hot set, start tier | lowering jitless: SLOWPATH / DEOPT in budget / PIN; lowering hybrid: DEOPT, then PIN + re-tier at the function budget (T4) | budget and exit counters (C-08) |
| GenInterp (candidate 5) | M5c, engine build | no | top tier if K-A3 trips (P04:316-318; P05:378) | no | n/a | full FeedbackVector |
| Lazy (candidate 6) | none: "no bytecode yet" state (H04-Q1, H04:78) | all | all | all | n/a | n/a |
| later kinds | appended to the list when built (C-03) | | | | | |

- **Kinds are appended when built, not reserved now.** The kind list becomes an append-only part (C-03): a new kind byte is a fill when its kind is built. Bytes 5 and 6 are the planned next values, written in the text as candidates. No code changes now.
- **Variants are not kinds.** Region code + generic sibling (A5v), CPU-level variants and the jitless and hybrid builds of one function are variants of AotOpt in one code set (section 3.1). The debug bytecode copy is a variant of Interp (section 3.2). This keeps the kind space small.
- Kind 5 fixes contradiction C5 (below): no kind names M5c today (`crates/ep9_vm/src/dispatch.rs:21-32`), but P05:378 makes M5c the PIN base if K-A3 trips.

### 1.4 Lowering class of a code object

- Guard lowering differs by mode (H32:25-33). Jitless code needs SLOWPATH variants, site budgets and PIN. Hybrid code deopts and re-tiers.
- A unit does not record which lowering its code used. The header fields (H35:44-58) and the header layout (H35:94-104) have no such field. `ep9c` builds per mode (P05:296-300).
- So a hybrid-built unit can reach a jitless isolate. There, a DEOPT guard with no re-tier is the "forever deopt" of P05:311.

Design: every code object carries a 1-byte **lowering class**. The name is not "build class S/H/J", because "class S" already names a measurement class in this repo (H15:62; SCP:103; DL:2778).

| Lowering | Value | Built for | Guard lowering | Needs at run time | Accepted by |
|---|---|---|---|---|---|
| `jit` | 0 | in-memory JIT code (T1, T2, hybrid re-tier) | H32 JIT column; HeapConst allowed (H34:41) | codegen permitted | JIT and hybrid presets; never stored in a unit |
| `hybrid` | 1 | hybrid units | H32 hybrid column (DEOPT, then re-tier) | `produce` not empty | hybrid preset |
| `jitless` | 2 | jitless units | H32 jitless column (SLOWPATH, budget, PIN) | nothing | jitless and hybrid presets |
| later | 3+ | appended when built (for example VERIFIED region code, H32:33) | | | refused until appended |

- Loader rule: a variant whose lowering is not in `accept` is skipped. The function keeps its next code (bytecode at worst). The skip is counted by reason, as every other fallback is (H35:90). A unit variant that names `jit` is skipped in every preset, because `jit` is never stored in a unit; so `jit` in the hybrid list admits only runtime code.
- One filter (MODES-D13): `select()` applies this `accept` test to every code object in the code set, unit code and runtime JIT code alike (section 3.2). There is no second rule for unit objects.
- One unit may carry a `hybrid` and a `jitless` variant of a hot function. The function table holds both (C-17).
- Home: the list (append-only part) and the acceptance rule go into H32 (C-07). The `hybrid` value and its rule go into the hybrid part, frozen at K-A4P (section 5.4). The byte goes into the H35 function table at A2 (C-17).

### 1.5 Contradiction C1: PROFILED facts in JIT code

- The mode table gives JIT mode no PROFILED source (P02:129). The JIT builder reads RUNTIME plus hints only (P04:226-227). But the guard table has a JIT-mode PROFILED row (P04:245; H32:29). And the H33-Q2 default lets instrumented T1 keep "PROFILED facts ... as lower-precedence inputs" (H33:81).
- Decision: the JIT-mode PROFILED row is the rule for **any JIT-compiled code that has a PROFILED input**. In v1 that is only hybrid re-tier code, which reads the unit's facts section. Pure JIT mode reads no PROFILED facts in v1. A "warm start" profile beside a code cache is reserved (F17). P02:129 gets a plan clarify for the hybrid row: "RUNTIME after re-tier; unit PROFILED and HINTED at lower precedence (H33-Q2)".

---

## 2. Layers and the one-optimizer pipeline

### 2.1 Crate layers: unchanged, plus three seams

The layering of P02:236-259 stays. Compilers sit above `ep9_vm` (P02:245-249).

```
 binaries   ep9 (JIT | jitless)                                    ep9c (`aot`; not shipped, P05:427)
 runtime    ep9_runtime  ep9_node  ep9_inspector  ep9_napi              |
 facade     ep9 (+ unit loading API) -- builds ExecOptions + TierHooks -+   ep9_aot: module graph, hot set,
                 |                                                      |     .ep9prof REBASE, .ep9h join
 compilers  ep9_baseline (T1 | baseline-AOT)   ep9_opt (T2 [T3] | optimized-AOT)
                 |   in: CompileRequest   out: CodeDesc          <-- SEAM S-1
 codegen    ep9_masm (finalize_jit | finalize_unit)      ep9_unit (loader | writer)
                 |
 engine     ep9_vm: T0, ICs, FeedbackVector, deoptimizer, fuse registry, dispatch slots,
                    code set + select() + tier manager, pc->code registry, native entry
                    calls UP only through TierHooks (fn pointers)  <-- SEAM S-2
 formats    ep9_bytecode   ep9_facts (FactSnapshot, L2 codecs, merge)  <-- SEAM S-3   ep9_hints
 base       ep9_heap  ep9_value  ep9_platform
```

| Seam | Shape (proposal) | Why | Today |
|---|---|---|---|
| S-1 CompileRequest / CodeDesc | Both compilers take one request value and return one code description. Both finalizers take a CodeDesc | One deopt reader, one dependency list and one install path for JIT code and unit code | `ep9_masm` with `finalize_jit` and `finalize_unit` is planned (H34:9, H34:27-28) but does not exist yet (H34:62) |
| S-2 TierHooks | A struct of function pointers inside `ExecOptions` (never in `IsolateConfig`, section 1.2): `on_hot(fn, reason)`, `on_osr(fn, offset)`, `on_invalidate(code)`. A missing hook is a no-op | `ep9_vm` cannot link the compilers. This is the `CompilerHooks` pattern (`crates/ep9_vm/src/compiler.rs:43-56`) | none: no hook from `ep9_vm` to a compiler, no hotness counter after the vector exists (`crates/ep9_vm/src/function_info.rs:78-80`) |
| S-3 one merge | `ep9_facts::merge(sources) -> FactSet`, used by the JIT and by `ep9_aot` | P04:226-229 draws two merge boxes today. `ep9_facts` depends on `ep9_bytecode` only (H16:98), so both sides can call it | none |

A jitless build installs no TierHooks. Its tier manager still does unit load, PIN, deopt and fuse swaps. This keeps H27 true: the jitless build keeps the loader and the deoptimizer (P02:184; P08:116).

### 2.2 The pipeline and its three mode branches

```
 [B1 fact selection]   ExecPolicy.facts + build target choose the sources
   PROVEN    analysis in ep9_opt                         every mode
   RUNTIME   FeedbackVector -> main-thread snapshot      JIT; hybrid re-tier            (H30-Q4 default, H30:95)
   PROFILED  .ep9prof -> REBASE -> FactSnapshot          ep9c builds; hybrid re-tier (lower precedence)
   HINTED    .ep9h -> join -> FactSnapshot               ep9c builds; JIT only if K-A4H keeps it (P02:129)
   VERIFIED  own region table (reserved; H30:96)         none until A5
      |
      v  ep9_facts::merge: per fact kind RUNTIME > PROFILED > HINTED; fused HINTED kinds stay (H30:51)
 FactSet
      |
      v
 [B2 guard creation, in the one graph builder (H30:64)]   GuardTable value (H32 column) + lowering class
   Guard(kind, fact, policy)  policy = DEOPT | SLOWPATH-capable (DEOPT in budget, then SLOWPATH) | DEOPT-then-retier
   Assume(fuse, form)         form   = GuardFuse load + compare | InvalidatingFuse dependency (H30:49)
      |   SSA with Trust{Proven, Predicted(source, confidence), Unknown}; ref nodes only (H30:41-47)
      v
 passes: read Guard.policy as data. No `cfg`, no mode name.
   DEOPT guard           -> narrows the type after it (failure leaves the function)
   SLOWPATH-capable guard -> a diamond: fast arm (narrowed) + generic arm, joined by a phi (not narrowed after)
      v
 ep9_masm stream + abstract relocations (H34:32-41)
      v
 [B3 finalizer]
   finalize_jit  -> W^X range; HeapConst allowed; CodeObject + CodeEntry      (`jit`)
   finalize_unit -> text (0 relocations) + rodata + RW template + function-table variant   (`aot`)
```

Why B2 sits in the graph builder (F3): a SLOWPATH guard rejoins the main path ("generic IC call inside the AOT code", H32:39-43), so code after it may not assume the fact. A DEOPT guard may. A pass that does not know the policy either assumes a fact that a later SLOWPATH breaks, or must copy the rest of the function as generic code (the size cost of H32-Q2, H32:89). H30:65 and H32:61 already require "the guard policy is chosen per mode in the IR, not in the backend".

```
 CompileRequest = { function: FunctionRef (unit hash, id) + bytecode, FactSet, code kind,
                    target: jit | unit, GuardTable, lowering class, variant tag, DepSink }
 CodeDesc       = { function, kind, variant {lowering, region|sibling, cpu level},
                    text, exits[], polls[], calls[] (lazy-deopt return points),
                    resume map (reserved), osr entries (reserved),
                    unwind (CFI / UNWIND_INFO, H14:92; P02:185), positions (pc -> bytecode / position id, H24:48),
                    perf_name (perf map / jitdump, H24:74), stack maps (empty in v1, P03:53; reserved for a moving heap),
                    rw layout {exit counters, budget counter, IC cell refs, link cells},
                    deps {fuse refs, inlined FunctionRefs + digests}, cpu_required }
```

Rules:

| ID | Rule | Why | Check |
|---|---|---|---|
| P-R1 | `ep9_baseline` and `ep9_opt` have no `cfg(feature = "jit")` or `cfg(feature = "aot")` outside the finalizer call. Passes read `Guard.policy` and `Assume.form` as data and never read a mode | Mode branches stay in B1-B3 (P04:237: about 276 AOT branches in 237k lines in Dart) | dep-guards row when the crates appear |
| P-R2 | JIT graphs obey the H30 ref-node rule too. HeapConst appears only inside `finalize_jit` | The IR has no raw heap constant (H30:61). This keeps "re-finalize a JIT request as a `hybrid` unit" open (F21) | unit writer rejects HeapConst (H34:41); IR verifier |
| P-R3 | The `DepSink` is the one list of assumptions. `finalize_jit` registers it with the fuse registry; `finalize_unit` writes it into the function-table variant | One list, two consumers; the loader can then check deps per variant (section 5) | A2 golden tests |
| P-R4 | A CompileRequest is a value. The same request with `target = unit` gives the same bytes | Determinism of units (H2 style) | goal only, tested at A3; not frozen |
| P-R5 | A pass that moves code across a guard keeps the guard's policy semantics: it never moves a use of a narrowed value above a DEOPT guard, and never treats the value as narrowed after a SLOWPATH-capable guard | F3 | IR verifier rule (M7, T2) |

---

## 3. Code objects, dispatch slots and transitions

### 3.1 Objects

```
 FunctionInfo (per isolate, shared by realms; repr(C); "New fields go at the end", crates/ep9_vm/src/function_info.rs:54-55)
 +--------------------------------------------+
 | slot: DispatchSlot @0                       |  kind u8 @0; bytes 1-7 are Rust padding today; entry @8
 |   ... existing fields (:59-80) ...          |  (crates/ep9_vm/src/dispatch.rs:77-84, asserted crates/ep9_vm/src/dispatch.rs:122-125)
 | + code_set: *const CodeSet   (new, at end)  |  null while the function has only bytecode
 | + hotness: Cell<u32>         (new, at end)  |  H23 budget units after the vector exists
 +--------------------------------------------+
            |
            v
 CodeSet: [CodeRef; small, typically <= 3 (est.)]  + Interp variants {normal, debug copy (H24-Q3)}  + pinned: bool (PIN, section 3.2)
            |
            v
 CodeObject (one per compiled body; text shared per process, RW per isolate)
   kind (slot byte), variant {lowering, region|sibling, cpu level}, origin {jit | unit id + fn index}
   state: Active | Invalidated   (PIN is a function state in the CodeSet, not an object state)
   entry, text range, meta -> exits, polls, calls, resume map, osr entries, positions, unwind   (the CodeDesc)
   rw   -> exit counters[], budget counter
   deps -> fuse refs, inlined FunctionRefs + digests

 pc -> CodeRegistry::lookup -> CodeEntry {kind, start, len, function, + code: CodeRef (new)}
```

- The slot stays the only hot read. A call reads one slot, and no code patches a call site (H14:69).
- Slot bytes 1-7 are Rust padding (`kind: Cell<u8>` then `entry: Cell<usize>`, `crates/ep9_vm/src/dispatch.rs:77-80`). This design does not reserve them. A field put there later moves no offset (`KIND_OFFSET` 0, `ENTRY_OFFSET` 8, `crates/ep9_vm/src/dispatch.rs:83-84`), so the room exists without a contract change now.
- `CodeEntry` has no metadata handle today (`crates/ep9_vm/src/dispatch.rs:129-138`). The deoptimizer needs pc -> code object -> exit record -> exit counter. The handle comes with `DEOPT_META_VERSION` 1.
- Bytecode life: M-R7 (section 1.1).
- Cost: two words per FunctionInfo, plus a CodeSet only for functions that ever get native code or a breakpoint (est.).
- Contradiction C6: the spikes install unit code in an Opt slot (SCP:26; H14:67). Production rule: the loader installs AotBaseline or AotOpt (H35:73), never Opt. The Opt marker stays spike-only.

### 3.2 select()

```
 select(f, policy, session):
   if session.connected and f has a breakpoint:  return Interp(debug copy)       -- H24-Q3, H24:75
   for kind in policy.install (rank order):
     for obj in f.code_set where obj.kind == kind, obj.state == Active,
                 obj.variant.lowering in policy.accept (rank order),   -- every object, `jit` ones too
                 not (obj.kind == AotOpt and f.code_set.pinned),       -- PIN, MODES-D14
                 obj.deps intact (per isolate, section 3.5), obj.variant.cpu bits present:
       return obj
   return Interp(debug copy if session.connected and f has one, else normal)    -- or Lazy, once H04-Q1 builds it
 every event ends with:  f.slot := select(f, policy, session)   (one slot write, crates/ep9_vm/src/dispatch.rs:116-119)
```

- `select()` is a pure function. It can be tested with fake code objects and no codegen (C-15).
- The debug copy is a variant of Interp, so a fuse pop or a deopt during a session keeps the breakpoints. Breakpoints and `debugger` statements still deopt to T0 before the pause (H24:22).
- **PIN (MODES-D14)** = set `pinned` in the function's code set, then `select()`. PIN is a function state, not an object state. It excludes every AotOpt variant of the function (CPU levels, region and sibling, `hybrid` and `jitless` builds), also AotOpt objects installed after it (T1). In jitless this gives AotBaseline, else GenInterp, else Interp. That is the H32 target "aot-baseline (-> interp if the function has no baseline-AOT)" (H32:42-43) plus the K-A3 case "M5c or T0" (P05:378). In jitless PIN is final for the isolate (H32-Q4 default, H32:91). In hybrid it holds until the re-tier policy lifts it (T4; the lift rule is open, section 9). This fixes C5. Marking only one AotOpt object would not do: `select()` would pick another Active AotOpt variant before AotBaseline, and the function would keep taking speculation exits past its budget.
- A jitless InvalidatingFuse pop "swaps the slot to aot-baseline or interp" (P04:120). With `select()` this is the same rule, with no special case.
- **Counter sessions (MC-w15-17, MC-w16-13).** A counter session (coverage, a profile run) is handled like a debugger session: `select()` picks code that has the counters, or the session mode joins the codegen-flag hash (C-18), so cached code without counters never runs in it. Why: instrumented runs can silently use cached code (Deno lost coverage counters to the V8 code cache). The snapshot half of the same finding is the F19 note in 6.2.

### 3.3 State machine (per function)

```
                         unit load (H35 step 8)                    hot: hotness >= T1 threshold (~500 units est., P04:104)
          +-------------------------------------------------+    +------------------------------------------+
          |                                                 v    |                                          v
     [Interp] <------- frame deopt (eager or lazy) ------- [AotOpt] --- re-tier trigger (hybrid) ---> [Baseline] --- T2 threshold ---> [Opt]
       ^  |   \                                             |  ^      calls/budget >= 1,000 est.        (full feedback)                 |
       |  |    \--- hot (JIT) ---> [Baseline] -------------------------------------------------------------------------> [Opt]      |
       |  |                                                 |  |      or fn exits >= budget (16 est.)                                  |
       |  +-- unit load --> [AotBaseline] -- hot (hybrid) --|--|--------------------------------------------------------> [Opt]      |
       |                         ^                          |  |                                                                     |
       |                         +---- PIN -----------------+  +-- jitless: site budget -> SLOWPATH (stays in AotOpt)             |
       |                         (code_set.pinned := true; select())                                                                 |
       |                                                                                                                             |
       +------- fuse pop: dependents := Invalidated, lazy deopt of their frames, select() <------------------------------------------+
                debugger attach: see T10 (per code kind, not one path)
```

| # | Event | JIT | hybrid | jitless | Slot write | Contract |
|---|---|---|---|---|---|---|
| T1 | unit load | n/a | add AotOpt/AotBaseline objects; select() (a PINned function skips the new AotOpt objects) | same | yes | H35:73; C-17 |
| T2 | T0 gets hot | TierHooks.on_hot -> T1 -> T2 (lowering `jit`, in the JIT accept list) | same (`jit` is in the hybrid accept list) | never re-tiers (P04:118) | yes | H14; S-2 |
| T3 | AotOpt gets hot | n/a | budget counter -> T1 -> T2 | never | yes | H33:29; C-08 |
| T4 | AotOpt speculation exits pass budget (O2: environment exits do not count) | n/a | PIN + start re-tier (H33-Q4 default, H33:83); PIN holds until the re-tier policy lifts it | PIN, final (H32:40-43; H32-Q4) | yes | C-07, C-08 |
| T5 | eager deopt | frame -> T0; code stays (SCP:128); exit counter | frame -> T0; code stays until T4 | SLOWPATH or DEOPT in budget (H32:37-47) | no | H29, H32 |
| T6 | fuse pop | dependents Invalidated; lazy deopt; select() | same | GuardFuse code takes the slow path; InvalidatingFuse as JIT (H31:28-29) | yes | H31, H29 |
| T7 | OSR into optimized code | T0 -> Opt at a loop header | same | n/a | yes + frame copy | H29-Q6 |
| T8 | OSR out of AOT code | n/a | lazy deopt at the `JumpLoop` poll, then T7 | n/a | no | C-04 |
| T9 | OSR into AOT code | n/a | n/a | reserved: osr-entry record | no | C-04 (reserved) |
| T10 | debugger attach | JIT code: InspectorSession is an InvalidatingFuse -> Invalidated; lazy deopt; re-tier later builds session-aware code | JIT code as JIT mode; AOT code as jitless | AOT code reads InspectorSession as a GuardFuse at polls ("treat every interrupt poll as a heap effect point", H24:22): objects stay Active; frames on the stack at attach deopt lazily ("AOT frames lazily", H24:22) | yes | H24, H31; C-22 |
| T11 | breakpoint set / cleared | select() picks or drops the Interp debug copy | same | same | yes | H24-Q3; C-22 |

- **Contradiction C4 (hybrid deopt).** P02:130 and P04:119 say "DEOPT, then instrumented re-tier". P05:341-343 and H33:32-34 draw "DEOPT -> T0 frame" and "re-tier by calls". Rule: the frame goes to T0. The function keeps AotOpt, because most calls still win. When the exits pass the function budget, the function is PINned (no AotOpt variant is selected) and the re-tier starts (T4). So the function cannot deopt on every call while T1 compiles. The budget number freezes at the A4 gate (H32-Q5, H32:92).
- **Why T10 differs per kind.** Fuses are one-way (H31:40). If attach marked AOT code Invalidated, one attach in jitless mode would send the isolate to T0 for good. H24:22 lets AOT code keep running during a session. The DebuggerBindingWrite fuse is separate: code that used PROVEN binding facts depends on it as an InvalidatingFuse in every mode (H31:24).

### 3.4 One native entry

- Today the shipped call path cannot enter native code. T0 `call_fast` sends any non-Interp kind cold (`crates/ep9_vm/src/interp/ops.rs:391`). `call_with` throws "not implemented" (`crates/ep9_vm/src/call.rs:467-469`). The native entry exists only under `spike-native` (`crates/ep9_vm/src/lib.rs:102-103`), which implies `jit` (`crates/ep9_vm/Cargo.toml:32`). Since M5-P0 (C-18) the 3 native `Cx` fields, the `RuntimeTable` type and the layout export `ep9_vm::layout` are in every build; the table itself and the functions it calls stay behind `spike-native` (`crates/ep9_vm/src/layout.rs:7-11`).
- Rule (H27 clarify, C-10): the production native entry, the tier manager and the deoptimizer build without `jit`. `jit` adds producers only. Every call path branches on "Interp or not". H04 already says "every call path checks the slot kind first" (H04:52). So a new kind byte later needs no caller change.
- Rule (MC-w15-6): a RuntimeTable entry calls only non-recording generic functions, so native code records no feedback (`crates/ep9_vm/src/native.rs` "records none"). A T0 cold body passes its feedback site in; a generic function never finds a site by itself (today `property::instance_of` finds the `TestInstanceOf` site of the top frame and records there, so no RuntimeTable entry may reach it).
- Build: M5 (T1) and A2 (loader). Nothing in M1.

### 3.5 Fuses across modes and realms

```
 FuseRef (fills H31-Q1)            READ IN CODE                              ON POP
   Static(index)   H13 list        GuardFuse: load via RootIndex            GuardFuse code: slow path, no deopt
                   (P04:262)       (per realm: through the current realm)   InvalidatingFuse dependents:
   Dynamic(desc)   per shape,      GuardFuse: load via LinkSlot (H34:37);     Invalidated + lazy deopt (H29) + select()
                   class, enum,    link cell resolved at load or first run
                   proto validity  (H35:88)
                   (ProtoRef),
                   A5v F1-F8
```

- **Why before M1a.** H31 says "Fuses have stable indices, so a unit and a snapshot name a fuse by index" (H31:90). The H31-Q1 default repeats it (H31:115). But H17 registers "the constness bit + replacement watchpoints as InvalidatingFuses (H31)" (H17:132). Those fuses exist per shape and per field, only at run time. H17 freezes at M1a (H17:7; AR:37), before H31 (M1 exit). TS-derived CHA and enum folds have the same need (H31:37; H31:88). So the FuseRef form must allow dynamic fuses before M1a. This fixes C8.
- Descriptor kinds are an append-only list (section 5.4). A new kind is a fill when it is built.
- **Prototype-chain validity (MC-w13-11).** M4-P1 built validity cells (VALIDITY_CELL, kind 79; H17 validity row). They are a new way to invalidate: `invalidate` clears a cell and its users and has no registry hook (`crates/ep9_vm/src/validity.rs:270-276`). An invalid cell stays invalid, and the next IC miss makes a new cell (`crates/ep9_vm/src/validity.rs:33-34`). So a LinkSlot that binds the cell itself goes stale after the first invalidation; the IC data path rebinds at its next miss, but compiled code would not. Rule: the first named descriptor kind is "prototype-chain validity by ProtoRef". The descriptor names the prototype (a ProtoRef), not the cell. The fuse registry keeps the dependents, not the cell. ADR-A0 names it in the H31-Q1 list and adds one sentence to the H17 validity row (C-01, C-02).
- No new relocation kind: a dynamic fuse word is read through `LinkSlot` (H34:37).
- **Realms (F15).** H31-Q2 makes intrinsic fuses per realm (H31:116). A FunctionInfo and its slot are per isolate and shared by the realms (H22:23; H22:87; a callee of another realm runs through `slow::call`, `crates/ep9_vm/src/interp/ops.rs:394-397`). So a code object's InvalidatingFuse dependency is per isolate: a pop of that fuse in **any** realm invalidates the code object. Code that must survive a pop in another realm reads the fuse as a GuardFuse through the current realm.
- Load-time check per code object (A2): the loader resolves the fuse refs of each variant at steps 6-7 (H35:70-72). A fuse that already popped marks that one code object Invalidated. The unit still loads.

### 3.6 Deopt across modes

```
 guard fails -> DeoptExit(id) -> shared entry (ep9_vm, every build; P04:288)
   -> registry lookup(pc) -> CodeEntry.code -> CodeObject.meta -> exit record at the pc offset
   -> rebuild the canonical frame in place (placement (b)) -> exit counter[i] += 1 (CodeObject RW)
   -> resume at the bytecode offset: T0 (always legal), or resume-capable code of the same function (gated part)
   -> tier manager reads counters: JIT recompile rule | hybrid PIN + re-tier | jitless SLOWPATH / PIN
 lazy deopt (H29:37): the return-address swap lands at a CALL RETURN POINT, so every call site of optimized code
   needs a record with value locations, beside the exit and poll records.
```

Exit-record content. Only tag values are reserved now. The record format (bytes per exit, H29-Q1), the location encoding (H29-Q2) and the swap rule (H29-Q4) stay with Spike C and the month-6 lazy-deopt probe (H29:92-95; SCP:30; DL:2778).

| Field | v1 values | Later (append-only list) | Enables |
|---|---|---|---|
| record tag | exit, poll, **call** (return point) | osr-entry; more by append | T9 OSR into AOT; later record kinds |
| pc offset, bytecode offset | as H29:27-28 | | |
| exit reason | guard kind + fact source (format by Spike C) + a class bit {speculation, environment} (O2) | | deopt-reason feedback into the next build (F14); only speculation exits count toward PIN (T4) |
| value location tag | 0 reg, 1 stack, 2 const-index | materialize, duplicate, more by append | escape analysis (F10) |
| inline-chain entry | frame kind "bytecode frame"; function = index into a per-code FunctionRef table (`crates/ep9_facts/src/facts.rs:37-45`) + bytecode offset | builtin continuation, construct stub, more by append | builtin inlining (F9); cross-unit inlining (F11) |
| exit counter | index into an RW array per code object (H29-Q5 default, H29:96) | | |
| resume rule | T0 at the bytecode offset (H29:36) | "or resume-capable code of the same function" (gated part, A4) | deopt into AotBaseline in jitless; native generators (F12) |

- Spike C refuses materialize (SCP:127). That is enough for the M1 gate. Its exit records do not use the tags above (MODES-CHECK-w16 MC-w16-3): SPIKEC-4 built its own form, where the value kind (tag bits 2-3) is 0 machine register, 1 native stack slot, 2 VM frame slot, 3 const (here 2 = const-index), and decode does not check tag bits 5-7 (`spikes/spike-c/src/deopt_meta.rs:13-18`, `spikes/spike-c/src/deopt_meta.rs:294-306`). It is a probe: the record format and the location encoding (H29-Q1, Q2) stay with SC, as the exit-record paragraph above says, and Spike C's form choices go to SC as input (MC-w16-21).
- **Exit classes (O2).** A speculation failure (a guard whose fact did not hold) is a "speculation" exit. A lazy deopt from an outside event (a fuse pop from another realm, a debugger attach, T6 and T10) is an "environment" exit. Only speculation exits count toward the function budget and PIN, so an outside event does not punish stable code. JSC made the same split in 2026 ("Only speculation failures mark functions as Unstable", WebKit commit 1837781c8a). ADR-A0 names the bit (C-04); Spike C fills its format.
- **Exit-reason key (MC-w15-19, MC-w16-13; F14, with H29-Q1).** An exit reason is keyed by (site, inline chain, guard kind), never by the site alone. Why: with a site-only key, one inlined callee's exits change how another callee inlined at the same site compiles; JSC had DFG/FTL crashes from this (WebKit 1af9d482aa, 2026-10-22). SC writes the key into the H29-Q1 fill (month 6).
- **Feedback slot (MC-w13-7; a note for H29-Q1, not a rule).** The exit record names the feedback slot of its guard. Reason: a recompile from the same feedback makes the same guard again when T0 left the slot implicit (an int32 `+` that overflowed, F-R9). The H32 budgets and exit counters already bound that loop. The slot in the record lets F14 feed the reason back.
- **Positions inside inlined callees (F11).** H24 R2 says positions come from the frame's offset slot, and the walker never needs a native pc (H24:29). H14 finds a handler "for each bytecode frame" from that offset (H14:73). With inlining, one physical frame holds several bytecode frames, and one offset cannot name them. H29 already requires an inline frame chain per exit (H29:30). Rule (C-22): in an optimized frame, the offset slot holds a bytecode offset or a **position id** that the code object maps to an inline chain (function + bytecode offset per frame). The walker, Error.stack and the handler search expand the id. They still need no native pc. The rewrite is accepted (MODES-D2): H24 and H14 are Accepted, not Frozen, so it is pre-freeze churn with data (H29:30).

---

## 4. Facts and profiles across runs

### 4.1 Artifacts

| Artifact | Layer | Content | Key | Version | Freeze |
|---|---|---|---|---|---|
| FactSnapshot | L1 | header {bytecode ABI, unit hash, one source}; per-slot records (H16:76-89) | (unit hash, function, slot) (`crates/ep9_facts/src/snapshot.rs:53-71`) | `FACT_SCHEMA_VERSION` 1 (`crates/ep9_facts/src/lib.rs:53`); one bump in month 6 (DL:2756) | M1 exit (H16) |
| L2 side tables | L2 | function records (calls, back edges, exits by reason and class, param/return repr), per-case hit counts, function digests, run count, class records (H37), tier hints = per function {priority u8 with a "runs once" value, stability tri-state unknown / stable / unstable, exit counts per class} (O3; format at A3) | SlotKey or function id of the same snapshot | own version per table, inside its container | with the container |
| VERIFIED region table (reserved) | own | region facts keyed by region version (H30:96) | region id + version | own | end of A5 (section 5.4) |
| `.ep9prof` | container | FactSnapshots + L2 tables (P05:290) | per unit hash | own | A3 (H16-Q5b, H16:137) |
| unit `facts` section | container | 0+ FactSnapshots (PROFILED, HINTED) + L2 tables | the unit | H35 container | A3 gate (H35:7) |
| `.ep9h` | input | hint records (P05:213-230) | (start, end, kind) + text hash | H37 | A4 start (AR:37) |
| CompiledUnit hint section | (unused) | stays in ABI 5 (H02:49, H02:53); v1 writers leave it empty | | H2 | M1 exit |

### 4.2 Flow

```
 RUN N (any mode)
   T0 / T1 / AotBaseline ICs -> FeedbackVector -> export_facts -> FactSnapshot{RUNTIME}   (ep9_vm/src/facts_export.rs)
     T0 runs the `profile.rs` record hooks only under `type feedback: full` (1.2); the IC-side
     counts (Literal, CloneObject, Construct; `literal_site.rs`, `construct_ic.rs`) stay under `ics_only`
     Call keys (FunctionInfo words) stay in the process; export maps them to FunctionRef (F-R8)
   CodeObjects -> exit counters + reasons, budget counters -> L2 function records
   `ep9 run --profile-out` -> .ep9prof { per unit hash: FactSnapshot(s), L2 tables, run count }
     a run that loaded a unit also copies the unit's PROFILED records of functions that have
     no RUNTIME record, marked "carried"; the run count is not bumped (O1, C-21)

 FLEET (A3+)  merge(.ep9prof, .ep9prof): commutative, associative. Bitsets OR (H16:89); saturating counts;
              IC state max; more than 4 cases -> megamorphic; run counts add

 BUILD (ep9c)
   .ep9prof --REBASE--> PROFILED FactSnapshot keyed to THIS unit hash
            match functions by digest (bytecode body + source-text hash), map old id -> new id, drop the rest
   .ep9h ----join----> HINTED FactSnapshot
   ep9_facts::merge(PROVEN, PROFILED, HINTED) -> FactSet -> compile -> unit
   unit `facts` section <- the PROFILED and HINTED snapshots + tier hints (for run-time use)

 RUN N+1 (hybrid)
   unit code starts; RUNTIME accrues in the FeedbackVector (unit IC cells point into it, H33-Q3, H33:82)
   re-tier: merge(RUNTIME > PROFILED from the unit > HINTED from the unit)
```

### 4.3 Rules

| ID | Rule | Fixes | Contract |
|---|---|---|---|
| F-R1 | Facts travel beside the CompiledUnit, never inside it. The container `facts` section (today `Hints`, kind 6, `crates/ep9_unit/src/codes.rs:238-239`) is the one home. The CompiledUnit hint section stays in ABI 5 and stays empty | C2: hints had two homes (P05:229; H02:49 vs H35:39). The CompiledUnit hash covers every byte but the hash and build id (`crates/ep9_bytecode/src/serialize.rs:450-457`), so a hinted unit would never match an unhinted training profile | H02 clarify (C-12); H35 at A2 (C-16) |
| F-R2 | FactSnapshot stays strict per unit hash: another ABI or unit hash refuses the whole snapshot (`crates/ep9_facts/src/snapshot.rs:189-198`; H16:100) | keeps H16 as built | none |
| F-R3 | Per-function staleness happens in REBASE, inside `ep9_aot`, before a FactSnapshot for the new unit exists | C3: P05:303 and H33:42 ("a function whose source hash differs ... ignores the profile") vs the whole-unit rule. Inputs exist: `source_start`/`source_end` per function (`crates/ep9_bytecode/src/unit.rs:861-863`) | H16-Q5b at A3 (C-21) |
| F-R4 | L2 tables live in their container. Adding an L2 table needs no `FACT_SCHEMA_VERSION` bump | room for PGO detail without a G8 rewrite of H16 | H16 fill (C-06) |
| F-R5 | The one month-6 schema bump (DL:2756) carries the H30-Q1 confidence byte per record, the H29-Q5 exit flag, the ElementsKind additions and fields for the keyed bits 4-9 (receiver, out of bounds, hole; `KeyedFacts` has only `{property, keys}` today, MC-w13-9; its `property.state` is live only for StoreKeyed, MC-w15-1). It also adds a per-site "ran" bit, written only inside the sampled window (MODES-D15, F-R9), for every site with an implicit bit: BinaryOp, CompareOp and the LoadKeyed implicit INDEX (MODES-D17). The LoadKeyed "ran" bit ships only together with the keyed receiver, out-of-bounds and hole bits, in this same bump: schema 1 drops those bits, so an out-of-bounds, hole or non-array LoadKeyed exports the same `keys {index}` record as a present int32 read (SPIKEC-3 test `facts::tests::keyed_records_do_not_show_bounds_holes_or_receivers`, `spikes/spike-c/src/facts.rs:211`), and a "ran" bit alone would make such a site look like an in-bounds int32 reader (MODES-D18). VERIFIED facts are **never** FactSnapshot records: FactSnapshot is keyed by (function, slot) (H16:82), VERIFIED by region version (H30:96), and any format change bumps the schema (H16:97). Source byte 3 stays refused and is only a name | C7: "HINTED ... at lower confidence" (H32:30) uses a field v0 lacks (H30:92). F6 | H16, H30 (C-05, C-06) |
| F-R6 | RUNTIME means "this run" only. PROFILED data never seeds a FeedbackVector. Instrumented T1 starts empty (H33-Q2 default, H33:81). Carried PROFILED records in a `--profile-out` file (O1) do not seed a vector either | profile lock-in: Leyden replay gave a 20-40% slower steady state (P05:304) | H33 (C-08); C-21 |
| F-R7 | Each count that T0 records or the exporter writes names its unit. `CallFacts.count` at a `Call*` site = sampled calls (a call in the first `CALL_WINDOW` (64) budget units after a budget reset, `crates/ep9_vm/src/profile.rs:96-102`, `crates/ep9_vm/src/profile.rs:177-179`); at a `Construct*` site = constructs of the cold path (`crates/ep9_vm/src/construct_ic.rs:22-23`). `LoopFacts.back_edges` = sampled back edges (a back edge that polls). `LiteralFacts.count` = results of the cold path, a lower bound of the allocations (`crates/ep9_vm/src/literal_site.rs:35-36`). A fleet merge adds counts of one unit only, never two units (H16-Q5b). H16 names the unit of all four counts (FACTS2) | MC-w13-2: the counts changed meaning in M4-P1; a hot-set picker (F13, F16) could get the scale wrong | H16 (C-06; FACTS2 text in M4-P2) |
| F-R8 | A Call key is never written out. A `Call*` site keys the native word of the callee's FunctionInfo (`crates/ep9_vm/src/profile.rs:631-642`); a `Construct*` site keys the constructor cell, held weakly (`crates/ep9_vm/src/construct_ic.rs:14-19`, weak: `crates/ep9_vm/src/construct_ic.rs:22-24`); one slot can be shared by both. The exporter branches on the site opcode, maps a FunctionInfo word to `FunctionRef::Js{unit hash, id}` or `FunctionRef::Builtin(id)` (`crates/ep9_facts/src/facts.rs:36-49`), and else writes the site as megamorphic. A host closure has no `FunctionRef`, so it maps to megamorphic | MC-w13-8: the word means something in one process only | H16 (FACTS2 tests: a shared call/construct slot; a periodic call pattern for the month-5 jitter, MC-w13-17) |
| F-R9 | "Implicit int32" means "int32 or never ran" (MODES-D10, option A for M1). T0 writes no INT32 bit at a BinaryOp or CompareOp site: an int32 pair writes nothing (`crates/ep9_vm/src/profile.rs:90-95`), and an int32 `+` or `-` that overflows stays implicit (`crates/ep9_vm/src/profile.rs:103-105`). A slot starts as int32 0 (`crates/ep9_vm/src/feedback.rs:172`), so a PROFILED file cannot tell a cold site from an int32 site. AOT treats an empty BinaryOp or CompareOp slot as "int32 or not run"; the H32 site budget bounds the cost when the site later sees other values (then it goes SLOWPATH). FACTS2 measured it on the bench set in M4-P2 (a measure-only build that sets a "ran" bit per site and counts function entries): of the empty sites, 14.7% of BinaryOp (124 of 846) and 10.4% of CompareOp (34 of 327) are cold, 0-36% per workload (ci-v0 the highest). Function call counts cannot stand in: 94 of the 124 cold BinaryOp sites (22 of the 34 CompareOp) are in functions entered 2 or more times, mostly dead branches. Decided (MODES-D15, option C): the month-6 schema bump adds a per-site "ran" bit written only inside the sampled window (no T0 cost outside it), and AOT compiles a never-ran site as generic and out of line, not as an int32 guard. Until that bump, option A holds. Decided (MODES-D16, P2-ORCH-10): the month-5 PROFILE step makes T0 record an int32 `+`, `-`, `Inc` or `Dec` that overflows, on the overflow path only (the hit path does not change), so Spike C's overflow guards stay unbacked until then. The record is `BinaryHint::INT32_OVERFLOW` ("int32 operands and a result outside int32", `crates/ep9_facts/src/bits.rs:283`), which `*`, `/` and `>>>` already write, not NUMBER (a non-int32 operand) (MODES-D16 as refined; MC-w14-19). Keyed sites have the same meaning: an INDEX-only LoadKeyed record means "int index or never ran" (the exporter adds INDEX to every LoadKeyed record, `crates/ep9_vm/src/facts_export.rs:417-418`), and a keyed fast path keeps the cold-miss rule, so a case new to its site still reaches the cold hook (`crates/ep9_vm/src/profile.rs:107-108`; MC-w14-6; KEYED kept it in M4-P3). The month-6 "ran" bit covers LoadKeyed too (MODES-D17), only with the keyed receiver, out-of-bounds and hole bits (MODES-D18, F-R5). The exporter module doc states the meaning (`crates/ep9_vm/src/facts_export.rs:69-85`) | MC-w13-6; 0 T0 cost (V9-D6). "Implicit never counts as seen" is rejected (section 11) | H16 (FACTS2 text), H30 confidence (C-05) |

### 4.4 Trust and precedence per mode

| Source | JIT | hybrid AOT code | hybrid re-tier code | jitless | Merge |
|---|---|---|---|---|---|
| PROVEN | no guard | no guard | no guard | no guard | always holds |
| RUNTIME | DEOPT + counter | n/a (built before the run) | DEOPT + counter | n/a | wins per fact kind (H30:51) |
| PROFILED | none in v1 (warm start reserved) | `hybrid` guard | guard, lower precedence | `jitless` guard | loses to RUNTIME of the same kind |
| HINTED | only if K-A4H keeps it; lower confidence | `hybrid` guard | fused kinds and lower precedence | `jitless` guard | fused kinds stay (H30:51) |
| VERIFIED | reserved (region table) | reserved | reserved | reserved | H30-Q5 (H30:96), M7 exit |

- A site with 4 guard failures (est.) stops using its hint in every mode (H32:46).
- **What the M1 gate measures (MC-w13-23).** SP-Facts (C5c) measures PROFILED facts under JIT lowering only: `Mode` has only `Jit`, and every guard deopts (`spikes/spike-c/src/ir/mod.rs:358-371`; SCP:28). Nothing measures jitless lowering (SLOWPATH, budgets). Since M4-P2 FACTS2 the exporter writes every kind T0 records (`crates/ep9_vm/src/facts_export.rs:9-16`): BinaryOp, CompareOp, call, keyed and the other kinds, so a PROFILED file holds arithmetic and call facts. The keyed IC state is live only for StoreKeyed (the present-element fast case writes the key half, KEYED M4-P3): at the other keyed kinds `Uninitialized` means "no state", not "never ran" (MODES-D17's "ran" bit is the only "never ran" signal), and a keyed `Monomorphic` exports no case (MC-w15-1; `ep9_facts` `IcState` docs, test `keyed_ic_state_is_live_only_for_store_keyed_present_elements` in `crates/ep9_builtins/tests/facts_export.rs`). C5c is still not a jitless AOT speed number. The H30 Freeze table says so (the model part, the SP-Facts note of MC-w13-23; ADR-A0, M4-P2), and the A4 gate names a jitless-lowering measure.

---

## 5. Versioning and compatibility

### 5.1 Identity ladder: drop the smallest thing that fails

```
 LEVEL        CHECKS                                                     ON MISMATCH          WHERE
 container    magic, container version, header size, section table       LOAD ERROR           H35:46, H35:64
 integrity    content hash (+ signature)                                 LOAD ERROR           H35:47, H35:65
 engine       build id, bytecode ABI                                     SOURCE               H35:48-49, H35:66
 native       value, object layout, barrier, builtin ABI, page, frame,   BYTECODE             H35:50-57, H35:67
              IC, deopt meta, fact schema, CPU, target
              + VM_LAYOUT version, runtime table length, codegen flags,  BYTECODE             new (C-18)
                root-shape table hash
 ---- per function: new at A2-A3, R2R style -------------------------------------------------------------------
 variant      lowering not accepted; CPU bits; fuse already popped;     skip THAT variant;   C-17
              deps (inlined callee digests); snapshot binding           select() picks next
 profile      unit hash (FactSnapshot, strict) | function digest        drop snapshot |      F-R2, F-R3
              (REBASE)                                                  drop one function
 fact record  slot kind = unit.feedback_slots[i]                         drop the record      H16:100
 policy       auto | strict (`--unit-strict`: SOURCE/BYTECODE ->        LOAD ERROR           P05:282; H35:90
              LOAD ERROR); forms {o, ep9u}; every fallback counted
```

- A strict load mode already exists (P05:282; H35:90). The prior-art gap "no strict mode" is wrong at fc6ac78. Per-variant skip counts join the same counters.

### 5.2 Version rules per contract

| Contract | Home | Mismatch | Change |
|---|---|---|---|
| Bytecode ABI (H2) | `ABI_VERSION`; header +88 (H35:100) | SOURCE | none; no ABI bump in month 4 (DL:2787) |
| Native contracts | `crates/ep9_vm/src/versions.rs:41-69` (6 constants) | BYTECODE | `VM_LAYOUT_VERSION` 1 added in M5-P0 (C-18) |
| VM layout (new; `VM_LAYOUT_VERSION` 1 since M5-P0) | `Cx` offsets (`crates/ep9_vm/src/layout.rs:130-155`), FunctionInfo and slot offsets (`FI_CODE` `crates/ep9_vm/src/layout.rs:183`; `FI_SLOT_KIND`, `FI_SLOT_ENTRY`, `FI_REGISTER_COUNT`, `FI_PARAMETER_COUNT` `crates/ep9_vm/src/layout.rs:185-191`; and `FI_FEEDBACK`, which Spike C bakes and the layout export has since M4-P3 KEYED, ungated as `ep9_vm::layout` since M5-P0 (`crates/ep9_vm/src/layout.rs:194`; MC-w13-14, MC-w14-7), RuntimeTable length (append-only, H13; the keyed load and store entries that give Spike C keyed sites a generic path were appended by M4-P3 KEYED: `get_keyed` and `set_keyed`, entries 8 and 9, P2-ORCH-6; entry 10 `deopt` by M5-P0 P0-VM5, V10-D39; golden `crates/ep9_vm/tests/golden/runtime-table.txt`) | BYTECODE | new field in header reserved2 (H35:103) at A2 |
| Root-shape table (new, F22) | hash of the snapshot root shapes whose ShapeId unit text bakes as immediates (H35:23, H35:89; H30:45) | BYTECODE | 8 bytes in reserved2 at A2 |
| Codegen flags (new) | hash of the run-time flags that change emitted code (V8 has a flag hash in its code cache) | BYTECODE | 8 bytes in reserved2 at A2 |
| Engine build id | 16 B (H35:99, H35:118) | SOURCE | rule change (C-19) |
| Deopt metadata (H29) | `DEOPT_META_VERSION` 0 (`crates/ep9_vm/src/versions.rs:59`) | BYTECODE; 0 sends any text to BYTECODE (H35:115) | first real format = 1 (M5 or later) |
| Fact schema (H16) | `FACT_SCHEMA_VERSION` | snapshot refused; unit facts dropped (H35:56) | one bump in month 6 (DL:2756) |
| Container (H35) | `container_version` (H35:95) | LOAD ERROR | v2 at A2 start, while no unit exists (H35:119) |

**Why a VM-layout version.**

- **The set includes `FI_FEEDBACK` (MC-w13-14).** Spike C copied the private FunctionInfo layout to find the feedback field until M4-P4; since SPIKEC-4 (469d1b6) it takes `FI_FEEDBACK` and every other layout fact from the export (`spikes/spike-c/src/vm_layout.rs:30-35`), and since M5-P0 the POLY_IC handler half too: the handler word after a case's key is `POLY_HANDLER` (`spikes/spike-c/src/vm_layout.rs:122`, `spikes/spike-c/src/emit_a64/mod.rs:700`), which `ic.rs` exports with the case start, size and count (`crates/ep9_vm/src/ic.rs:201-207`); until M5-P0 it was a literal `+ 8` (C-18, IC_LAYOUT 3; MODES-CHECK-w16 MC-w16-4). Since SPIKEC-2 (d5a89d5) its emitter reads the native-word offset by name: each `eor` that decodes a native word uses `layout::NATIVE_WORD_OFFSET` (`crates/ep9_vm/src/layout.rs:233`; `spikes/spike-c/src/emit_a64/mod.rs:673`, `spikes/spike-c/src/emit_a64/mod.rs:1172-1173`), and `native_word_offset` refuses the compile when that offset is not one bit (`spikes/spike-c/src/emit_a64/mod.rs:245-255`). The facts still fail closed: a session checks the exported facts on live objects before any compile (`check`, `spikes/spike-c/src/vm_layout.rs:179-181`), and the exit emitter refuses a changed exit layout (`spikes/spike-c/src/emit_a64/mod.rs:1163-1165`). Rule: every layout fact that native code bakes moves into the layout export, `FI_FEEDBACK` included, and native code uses the named constants. M4-P3 KEYED made that export as `ep9_vm::native::layout` behind `spike-native` (8140b68; P2-ORCH-6, MC-w14-7): each constant Spike C copies by name, `FI_FEEDBACK` included. M5-P0 (C-18) moved it out of the gate as `ep9_vm::layout`. Each constant names the version that owns it: OBJECT_LAYOUT 5 (the object header, storage offsets and kind ids), IC_LAYOUT 3 (the feedback and handler offsets), FRAME 2 (the frame offsets), Value 1 (`HOLE`), and `VM_LAYOUT_VERSION` 1 only for what no other version owns (the `Cx`, FunctionInfo and RuntimeTable offsets, `FI_FEEDBACK` included; MC-w15-5; `crates/ep9_vm/src/layout.rs:13-17`). M4-P4 SPIKEC-4 switched `vm_layout.rs` to the export and uses the keyed RuntimeTable entries 8 and 9 as its generic keyed path (469d1b6; P3-ORCH-9).

- Native code bakes `Cx` and FunctionInfo offsets (`crates/ep9_vm/src/layout.rs:130-194`). Until M5-P0 no contract version covered them (`versions.rs` had 5 constants); since M5-P0 `VM_LAYOUT_VERSION` 1 does (`crates/ep9_vm/src/versions.rs:60-69`), and the header has no field for it yet (H35:94-104).
- Until M5-P0 two `Cx` fields under `cfg(feature = "spike-native")` sat before `realm`, so the offset of `realm` moved with a feature. Since M5-P0 the 3 native fields exist in every build (`crates/ep9_vm/src/interp/mod.rs:163-177`).
- Today the exact build id protects unit text, because it hashes `ep9_unit`'s features (`crates/ep9_unit/build.rs:108-115`). C-19 stops hashing `aot` and `jit` so that `ep9c` units can run in `ep9`. After C-19, two builds whose features move `Cx` could share one build id, and unit text would read wrong offsets. `VM_LAYOUT_VERSION` closes that gap without a feature list in the id.
- The later H35 relax of step 3 ("same bytecode ABI -> BYTECODE", H35:82, H35:162) is **not** a reason: BYTECODE drops native code (H35:80; P05:279), so stale text never runs there.
- Fix: make the 3 native `Cx` fields unconditional (`crates/ep9_vm/src/interp/mod.rs:163-177`), add `VM_LAYOUT_VERSION`, and add a header field. M5-P0 P0-VM5 did the first two with the removal of `Cx.proto_epoch` (DL:2753). The header field comes at A2 start (FREEZE-PREP check item, orchestrator decision M5-P0-ORCH-6).

**Build-id rule (C-19).**

- Today the id hashes the commit, rustc, the target and `ep9_unit`'s own Cargo features (`crates/ep9_unit/build.rs:108-115`; H35:118).
- `ep9c` turns on `aot` (P08:118); the runtime does not. So their ids differ, and every `ep9c` unit would load as SOURCE in `ep9`.
- Rule: the id hashes commit, rustc, target and a fixed list of features that change native contracts (today `become`, `crates/ep9_vm/Cargo.toml:28`). It never hashes `aot` or `jit`. Then the JIT build, the jitless build and `ep9c` of one commit share one id, and one unit can serve hybrid and jitless isolates (with `hybrid` and `jitless` variants). C-18 must land with C-19.

### 5.3 Container v2 (A2 start; container bump while no unit exists)

- Today every unknown value is a LOAD ERROR: non-zero reserved bytes (`crates/ep9_unit/src/header.rs:320-330`), unknown section kind and any section flag (`crates/ep9_unit/src/header.rs:390-394`). That is right for safety. But every new optional section then needs a container bump, and a bump makes every older unit a LOAD ERROR (H35:119).
- v2 adds:
  - `compat_flags` (unknown bit: ignore) and `incompat_flags` (unknown bit: BYTECODE), 8 of the 116 reserved2 bytes (H35:103);
  - extension section kinds >= 0x100, covered by the content hash; an entry flag bit 0 "native code needs this section" (unknown with bit 0: BYTECODE; without: skip);
  - section 6 renamed `facts` (F-R1);
  - reserved kinds `deps` (inlined units and their digests, F11) and `snapshot binding` (app snapshot hash, F19);
  - the header fields of 5.2 (VM layout, runtime table length, root-shape hash, codegen flags);
  - H35 S2 rodata row (H35:35) clarify: "safepoint maps" are empty in v1 (no stack maps while the heap does not move, P03:53) and reserved for a moving nursery (H29:19).

### 5.4 Room in the contracts: append-only parts and gated parts

- AR:76 makes it a rewrite when "a check now accepts what it rejected". AR:84 lets an ADR list parts with their own freeze points; after a part's freeze point, "Frozen part" rules apply. So a value that is turned on **after** its part freezes is a G8 rewrite. A part must therefore freeze at the gate that **ends** its build, or never close.
- The repo already has a part that never closes: H20 part 2, "the append-only lists ... None: the lists stay open for appends ... A new id ... is a fill ... not a rewrite, so G8 does not count it. Once added, an entry is held as the core is" (H20:7, H20:89). This design uses the same form. It needs no new process text and no "activation" kind.
- Negative tests for an open list use an "unknown value" probe at the top of the range (for example kind byte 0xFE, tag 15), never the next free value. Then an append keeps every existing test result, as AR:75 asks of a fill.
- An appended value of an open list names, in the same fill row, every frozen list that accepts it (for example the H32 preset accept lists: an appended lowering class names the presets that take it). So a frozen accept list takes a new value with no rewrite (MC-w14-3; F6, F7, F17).
- A list that is needed at M1 but grows later is split: the values named by its freeze point freeze with their part, and the rest is an append-only list. The H30 Q3 fill is the M1 list (frozen at M1 exit), then an append-only list of fused HINTED kinds (no codegen reads hints before A4). The H30 ref-node kinds are an append-only list too (MC-w14-4).

**Kind A: append-only parts (no close point).** A new value is a fill. An existing value never changes. A retired value stays reserved: it is never reused, and every reader treats it as unknown (MODES-CHECK-w16 outside item O3: V8 21e810452a, 2026-10-27, removed a feedback kind, a deopt reason and a dependency group, and renumbered).

| ADR | Append-only list | Unknown value today | Negative test |
|---|---|---|---|
| H14 | dispatch kind bytes (C-03) | `DispatchKind::from_u8` -> None (`crates/ep9_vm/src/dispatch.rs:43-52`) | high byte -> None |
| H32 | lowering classes (C-07) | loader skips the variant (A2) | class 0xFE -> skip |
| H34 | relocation kinds (C-09; F4). A new kind must be handled or rejected by both finalizers | finalizer rejects (when built) | unknown kind -> reject |
| H29 | record tags, value-location tags, inline frame kinds (C-04) | deopt reader rejects the code object (when built) | tag 15 -> code object refused, function runs bytecode |
| H31 | dynamic FuseRef descriptor kinds (C-01) | registry refuses -> variant skipped | unknown descriptor -> skip |
| H17 | cell kind ids (C-24). Until M5-P0 the `kinds.rs` module doc said the table "may change" because no kind id is persisted; since M5-P0 it says the ids are append-only (`crates/ep9_vm/src/kinds.rs:40-47`; C-24, V10-D5). M4-P1 appended 27, 28, 79 and 80; M5-P0 declared 81-83. AOT unit text that bakes a kind id (F18) and the identity ladder (drop the smallest thing that fails) need an append that keeps old units valid | none today (the table is private); C-18 decides at A2 whether an append is compatible with a unit | unknown kind id -> the IC or guard misses |
| H16 | IC handler kinds 1-8 (`crates/ep9_vm/src/ic.rs:156-186`; ALLOC_SITE 8 since IC_LAYOUT 3, M5-P0; C-24). They already behave append-only: each hit path tests its own kind | IC miss (`crates/ep9_vm/src/ic.rs:403`, `crates/ep9_vm/src/ic.rs:476`, `crates/ep9_vm/src/ic.rs:567`) | unknown handler kind -> miss |
| H16 | the CacheIR op list and the type set of `GuardType` (int32, number, string, symbol, bigint): the ops are append-only already (H16 "CacheIR op lists"); the `GuardType` type set is one too (MC-w14-19; the H16 text comes with C-06 in FREEZE-M1 batch 1) | no op list reader yet; a reader that meets an unknown op or type treats the case as a miss | unknown op or type -> generic path |
| H30 | fused HINTED kinds after the M1 list of the Q3 fill; the ref-node kinds (RootRef, IntrinsicRef, ShapeRef, CellRef, UnitConst today) (MC-w14-4) | no IR reader yet (Spike C has the five ref nodes, `spike_c::ir::RefNode`) | unknown ref node -> the unit writer refuses the graph (A2, planned); unknown fused kind -> not merged (A4, planned) |

**Kind B: gated parts.** Content with a known build gate. Each freezes at the gate that ends its build.

| ADR | Gated part | Built in | Part freeze point |
|---|---|---|---|
| H29 | resume in non-T0 code; osr-entry record format; materialize payload | A4 (with T2 AOT emit) | A4 gates (M8 exit, M09:182) |
| H30, H16 | VERIFIED region table format and ranking (H30-Q5) | A5, only if the D17 rule picks (d) | end of A5 (K-A5v decision, P05:386); M10 per M09:62 |
| H31 | A5v fuses F1-F8 (H31-Q4) | A5 | end of A5 (K-A5v decision) |
| H34 | T3 JIT finalize forms: the `finalize_jit` forms that only T3 code uses (T3 is optional, M9, N4; P04:56) | M9 | the gate that ends T3; if T3 is not built, the part is removed before its freeze point (MC-w14-14; H34:106) |
| H34 | T3 unit finalize forms: the `finalize_unit` forms that only T3-class unit code uses (optimized-AOT runs the "T2 (later T3) pipeline", P04:58; A5 needs a T3-class backend, P05:136), so the first unit user comes after M9 | after M9 (A5) | the end of A5; if A5 is not built, the part is removed before its freeze point (MC-w15-3; H34:107; split from the row above by MODES-CHECK-w16 MC-w16-1) |
| H32, H33 | budget and threshold numbers (4 per site, 16 per function, the re-tier threshold). The counter unit (the H23 budget unit: call + `JumpLoop`) and the rule (only speculation exits count, O2) were written by ADR-A0 in M4-P2 and are not part of this gated part (MODES-D12). They sit in these parts (MC-w14-2): the H32 counter unit and counting rule in the Policy part, M1 exit (H32:101); the H33 counting rule in the Rule part, M1 exit (H33:92); the H33 budget unit (Q1) and re-tier start rule (Q5) in the Hybrid part, K-A4P (H33:94). Whether the numbers become ExecPolicy defaults that change per platform without a G8 count is an A4 call (O4) | A4 | A4 gate (H32-Q5, H33-Q5) |
| H16 | T0 recording policy and IC tuning values (C-25, the full list there): the implicit cases, `CALL_WINDOW`, the sampled kinds, the keyed cold-miss and InstanceOf record rules, the stub cache size, hash and probe place, the instanceof walk group size. Today these values sit in the H16-Q2 and H16-Q3 cells (H16:133, H16:134), which freeze at M1 exit, so a retune after M1 exit would count toward G8 (MC-w13-15, MC-w16-9) | A4 (with the AOT consumers of the facts) | A4 gate |

**Kind C: the hybrid part (F9).** Hybrid ships only if K-A4P passes at M8 exit (P05:380). H33 says a failed K-A4P removes the hybrid rows, "a rewrite" (H33:99). So the hybrid-only rows freeze at K-A4P, as H33-Q5 already does for the threshold (H33:84): lowering `hybrid` and its acceptance rule (H32), the hybrid PIN + re-tier rule and the rule that lifts PIN (H32, H33), budget counters for re-tier (H33), the hybrid preset (text). If K-A4P fails, the part is removed before its freeze point, which AR:84 does not count.

- Decided (MODES-D1): this is the reading of AR:84. The orchestrator adds the rows to the AR §3.4 table in M4-P5, and an agent of another plan role approves them (C-13; AR:68). AR:84 already allows it, so this is a table fill, not a new AR §3.3 clause. [Revised 2026-10-28, M4-P5 (plan v9 §7 step 1, MC-w16-12): DOCS writes the rows and the orchestrator approves them; see the C-13 Route cell and the §8 M4-P5 row.]

---

## 6. Future-room seams

There are 24 items, F1-F24. The weekly MODES-CHECK (plan v9 rule 29) checks all 24. Cost = the cost of the change now (est.). "Text" = ADR or plan text only.

| # | Future optimization | Seam | Contract | Change now | Freeze point | Cost now |
|---|---|---|---|---|---|---|
| F1 | Speculative optimized-AOT that deopts to T0 (A4) | PI deopt metadata; RW exit counters | H29 | exit-counter home per code object (H29-Q5 default) | M1 exit | text |
| F2 | Hybrid re-tier that also catches hot loops | budget counter = H23 unit (call + `JumpLoop`, H23:33) | H33 | "call counters" -> "budget counters" (H33:23, H33:32) | K-A4P (hybrid part) | text |
| F3 | OSR out of AOT frames | polls are lazy-deopt points (P04:286) | H29-Q6 | rule: lazy deopt at the poll, then T0 OSR | M1 exit | text |
| F4 | OSR into AOT code (jitless hot loops) | osr-entry record tag; function-table field | H29, H35 | tag in the append-only list; field at A2 | tag: open list; format: A4 gates | text |
| F5 | PIN to a lower AOT body; jitless fuse swap | code set + select() | H14, H32 | name the code set; PIN = a function state (MODES-D14) | M1 exit | text + model tests (C-15) |
| F6 | Variants: CPU levels; jitless + hybrid in one unit | variant list; lowering class | H32, H35 | list now; function table at A2 | open list; A3 | text; 1 day at A2 |
| F7 | Sound typed regions (A5) | VERIFIED region table; region + generic sibling as variants; dynamic fuses | H30, H31, H32, H14 | gated parts | end of A5 | text |
| F8 | Dynamic fuses: TS CHA, enum folds, shape constness | FuseRef Dynamic via LinkSlot | H31, H17 | rule row H31:90; H17:132 | **M1a** | text |
| F9 | Inlined builtins (continuation frames) | inline frame kind list | H29 | append-only list | open list | text |
| F10 | Escape analysis, scalar replacement | location tags | H29 | append-only list; materialize format gated | open list; A4 gates | text |
| F11 | Cross-unit inlining; inlining at all in T2 and AOT | FunctionRef table in the chain; `deps` section; position id in the offset slot | H29, H35, H24, H14 | chain form now; position-id rule (C-22); section at A2 | M1 exit; A3 | text |
| F12 | Deopt into baseline code; native generators | resume map | H29 | gated part | A4 gates | text |
| F13 | PGO detail: per-case counts, function records, hot set | L2 tables | H16 | F-R4 text | M1 exit (text); A3 (format) | text |
| F14 | Deopt-reason feedback into the next build | exit reason; L2 exits-by-reason | H29, H16 | reason field named for Spike C (H29-Q1) | M1 exit | text |
| F15 | Incremental PGO (edit one function, keep the rest) | REBASE by function digest | H16-Q5b | none now | A3 | 0 |
| F16 | Fleet profile merge | `ep9_facts::merge` | H16-Q5b | none now | A3 | 0 |
| F17 | JIT warm start: tier hints and a profile beside a code cache | L2 tier-hint table; H32 JIT PROFILED row | H32, H35 | C1 clarify | M1 exit; A3 | text |
| F18 | Units that survive engine upgrades | build-id rule + VM layout version + root-shape hash | H35, versions.rs | fields at A2; relax step 3 later (H35:162) | A3 | 1 day at A2 |
| F19 | App heap snapshot + AOT code | `snapshot binding`; root-shape hash | H35 | reserve the section | A3 | in container v2 |
| F20 | Many independent units per isolate; engine-embedded units | unit-data slot rule (64 slots fixed at build, H35:155) | H35-Q9 | decide at A2 (section 9) | A3 | 0 now |
| F21 | Re-finalize a JIT CompileRequest as a `hybrid` unit (a new compile with another guard lowering; JIT finalize bakes immediates and addresses, H34:37-38) | CompileRequest value; P-R2 | H30, H34 | keep the ref-node rule for JIT graphs | M1 exit | text |
| F22 | M5c as the jitless top tier | kind 5 | H14 | append when built | open list | text |
| F23 | Lazy compile through the slot | kind 6 | H14, H04 | append when built | open list | text |
| F24 | More unit text than 128 MiB per process | unit placement | H7 | none now; decide at A2 with SIZE-REPORT data (C-11). H07:58 keeps direct calls between JIT code, units and stubs | A2 decision | 0 now |

### 6.1 Effect of the M4-P0 and M4-P1 work (MODES-CHECK-w13)

17 items are kept as they were. The others:

| # | Effect | Why | What this design does |
|---|---|---|---|
| F6 | widened | `ep9_math` gives the same bits on every CPU (`crates/ep9_math/src/lib.rs:1-24`), so CPU variants cannot change a Math result | none |
| F8 | kept, needs text | validity cells are the first per-object one-way cells | descriptor kind "prototype-chain validity by ProtoRef" (3.5, C-01) |
| F13 | widened data, narrowed meaning | T0 now records BinaryOp, CompareOp, TypeOf, keyed, Call, JumpLoop, ForIn and CreateClosure (H16:133); Literal, CloneObject and Construct sites keep counts (`literal_site.rs`, `construct_ic.rs`). But counts are sampled, "never ran" is lost, keyed bits 4-9 have no field, and the recording policy freezes early | F-R7, F-R9, F-R5 bump list, C-25 |
| F16 | narrowed (small) | sampled and exact counts need one unit; a fixed sampling period can hide a target (`crates/ep9_vm/src/profile.rs:100-102`) | F-R7; FACTS2 tests a periodic call pattern with the month-5 jitter |
| F18 | narrowed (small) | kind ids were called renumberable (the `kinds.rs` module doc until M5-P0; now append-only, `crates/ep9_vm/src/kinds.rs:40-47`) | append-only kind ids and handler kinds (5.4, C-24) |
| F20 | narrowed (small) | a feedback vector can name another unit's FunctionInfo (F-R8) | section 9 unit-unloading rule |
| F21 | widened | Spike C code has no heap constant and no absolute address; guards read IC cells (`spikes/spike-c/src/emit_a64/mod.rs:72-78`) | none |

### 6.2 Future-room notes from outside evidence (MODES-CHECK-w14 O4, O5, O6, O8; MC-w15-15; later: MC-w15-17, MC-w16-20, w17 O3)

Notes, not decisions; each names the item it touches.

- **Tier-hint fields (w14 O4; F17).** Two optional L2 tier-hint fields, "feedback settled at" and "inlined call sites". F-R6 holds for v1: the only seed prior art uses is a widening-only seed (SpiderMonkey `JitHints.h`, 9938d16c74, 2026-10-05), and it stays out of v1.
- **Cross-realm builtins (w14 O5; F9, F15).** An inlined builtin that makes realm-owned objects guards "callee realm = current realm" or uses the callee's realm (WebKit c2ccda5823, 2026-07-01).
- **Kernel W^X lock (w14 O6; F24, an A2 input).** Linux `PR_SET_MDWE` stops `mprotect` from adding exec, so "copy unit text, then flip it to RX" fails after the lock; the unit-placement and loader choice at A2 names an optional kernel W^X lock. A file-backed map was not tested.
- **Packed storage forms (w14 O8; F7).** Packed typed storage forms are not reserved; A5 decides with data.
- **Inlined WeakRef `deref` (MC-w16-20, MC-w17-4; F9).** An inlined `WeakRef.prototype.deref` must do AddToKeptObjects too: it adds its target to the kept set, as the builtin does (`crates/ep9_vm/src/abi/weakref.rs:92`; the `[[KeptAlive]]` list and the kept Set, `crates/ep9_vm/src/weakref.rs:3-4`, `crates/ep9_vm/src/weakref.rs:15-16`). Without it the target can die before the current job ends.
- **Builtin effects (MODES-CHECK-w17 outside item O3; F9).** Builtin effect facts (what an inlined or AOT-known builtin may read, write or call) live in their own table keyed by `BuiltinId`, never in a new `BuiltinDef` field: `BuiltinDef` is a core line of the Frozen H20 core (golden:595), so a new field would be a rewrite. Why: Bun reads one table of builtin effects (8d5672c); .NET 11 ReadyToRun code lost up to 20x on a builtin it could not see.
- **Per-run state after a snapshot load (MC-w15-17, MC-w16-13; F19).** After a snapshot loads, per-run state is set again, and unit code never bakes it: the random seeds (Node replayed cached random seeds from a snapshot) and the WeakRef `pending` flag (`crates/ep9_vm/src/weakref.rs:18`), which a heap snapshot must reset.

---

## 7. Contract changes

Kinds follow AR §3.3 (AR:72-77). All of these land before the named freeze, so G8 does not count them (AR:80). Draft ADRs take free edits (AR:31). Changes to Accepted ADRs need the orchestrator and the review record (AR:32, AR:63-65); a rewrite of an Accepted ADR also needs data (AR:65). Routing uses roles that plan v9 already has (section 8): **A0** = ADR-A0 in M4-P2 (Owns: H29-H34 Freeze-table and Open-question rows, H17 "Decide before M1a" proposals, H30-Q2; plan v9 §4.1, task 9 since MODES-D3); **FACTS2** = the facts exporter step 2 in M4-P2 (plan v9 §4.3); **FR0** = fuse registry v0, month 5 (plan v9 §0.4 and §10, H29-H34 row); **SC** = Spike C / SP-Facts fills, months 5-6 (plan v9 §10, H29-H34 row); **B1/B2/B3** = FREEZE-M1 batch writers, month 6 (plan v9 §10, FREEZE-M1).

### 7.1 Before M1a and M1 exit

| ID | ADR | Change | Kind | Route (plan v9) |
|---|---|---|---|---|
| C-01 | H31 (Draft) | FuseRef = Static(index) \| Dynamic(descriptor), both read through RootIndex or LinkSlot. H31-Q1 default filled with it. Rule row H31:90 -> "builtin fuses have stable indices; dynamic fuses are named by a descriptor". Descriptor kinds = append-only list; the first named kind is "prototype-chain validity by ProtoRef": the descriptor names the prototype, not the cell, and the registry keeps the dependents (3.5, MC-w13-11). Dependencies are per isolate; per-realm fuses in shared code are read as GuardFuses (F15) | Draft edit (fill + rule row) | Q1 default and Freeze-table list: **A0** (M4-P2): **done** (H31-Q1 proposal, the Freeze-table part "Dynamic descriptor kinds"). Rule row: **FR0**, before M1a |
| C-02 | H17 (Draft) | H17:132: constness and watchpoints register as InvalidatingFuses by Dynamic FuseRef (H31). No fuse-word assumption. One sentence in the H17 validity row (H17:135): compiled code depends on prototype-chain validity through a ProtoRef descriptor (C-01), never on a cell | clarify | **A0** (H17 representation/const proposal and the validity-row sentence, plan v9 §4.1 tasks 5 and 9); adopted by month-5 CONST/REP. The plan v9 §4.1 Owns line names H17 "Decide before M1a" rows "except the validity row", so task 9 needs an Owns change from the orchestrator before ADR-A0 starts. **A0 part done** (M4-P2: the H17 Proposals table and the validity-row sentence; the Owns change was made, M4-P1-MODES-DOCS) |
| C-03 | H14 (Accepted) | The kind list becomes an append-only part (section 5.4). Bytes 5 GenInterp and 6 Lazy are written as candidates, appended when built. "Each function has a cold code set; the slot caches select()". `CodeEntry` gets a code handle at DEOPT_META 1. Production unit code installs AotBaseline/AotOpt, never Opt (C6). Slot bytes 1-7 are **not** reserved (section 3.1) | rewrite (pre-freeze churn): the Decision kind list (H14:22, H14:69) changes form. Data: the H04-Q1 default (H04:78), the K-A3 rule (P05:378) and C6 (SCP:26) | **B1** (H14 freezes in batch 1, plan v9 §10); record rows with the batch |
| C-04 | H29 (Draft) | Freeze table: append-only lists of record tags (v1: exit, poll, **call**; osr-entry next), value-location tags (v1: 0-2) and inline frame kinds; gated part "resume in non-T0 code, osr-entry format, materialize" frozen at the A4 gates. Q5 default: RW counter array per code object. Q6: out of AOT = lazy deopt at the poll; into AOT = later. Q1 names an exit class bit {speculation, environment} (O2) and, as a note, the feedback slot of the guard (MC-w13-7). Q1, Q2, Q4 formats stay with Spike C and the month-6 probe | Draft edit | Freeze table and Q5/Q6: **A0**: **done** (M4-P2; also the Q1 exit class bit and the feedback-slot note). Q1/Q2/Q4: **SC** |
| C-05 | H30 (Draft) | Q1: confidence = 1 byte per record, PROFILED and HINTED (in the month-6 bump, DL:2756). Q2: VERIFIED = 3 stays a refused name; VERIFIED facts live in their own region table, never in FactSnapshot records (F-R5). Q4: one `ep9_facts::merge` for JIT and `ep9_aot`. P-R2: HeapConst only inside `finalize_jit`. Rule H30:65 text: "chosen in the graph builder when the Guard node is made" (F3) | fill | Open-question text of Q1, Q2 and Q4: **A0** (MODES-D3; plan v9 §4.1 tasks 3 and 9): **done** (M4-P2; Q2 filled, Q1 and Q4 proposals, and the Freeze table with the MC-w13-23 note). Rule text (P-R2, H30:65): **SC**, before batch 2 |
| C-06 | H16 (Accepted) | F-R4: L2 tables live in their container, no schema bump. F-R1 pointer. The month-6 bump adds no VERIFIED field; it adds the keyed bits 4-9 fields (MC-w13-9) and the sampled "ran" bit (F-R9, MODES-D15) for BinaryOp, CompareOp and LoadKeyed (MODES-D17), the LoadKeyed one only with the keyed bits (MODES-D18). The L2 tier-hint shape (O3). The count units (F-R7), the Call-key rule (F-R8) and the meaning of implicit int32 (F-R9). The CacheIR op list maps a case by its operation, and a `GLOBAL` store gets the action StoreGlobalField with the write barrier (P2-ORCH-9, Codex round 1 of M4-P2). The `GuardType` type set is an append-only list like the ops (MC-w14-19, 5.4) | fill (count units, Call keys, implicit int32: clarify of the H16-Q2 fill) | count units, Call keys, implicit int32: **FACTS2** (M4-P2): written in the exporter module doc (`crates/ep9_vm/src/facts_export.rs:69-109`); the H16 text is the section "Fact meanings" (one fill row, the M4-P2 Codex-fix docs agent, P2-ORCH-12; its review record is pending). The "ran" bit is decided (MODES-D15). Rest: **B1** with the schema bump (plan v9 §10, H16 row) |
| C-07 | H32 (Draft) | Lowering classes `jit`/`hybrid`/`jitless` as an append-only list; acceptance rule (1.4) with the accept list per preset: JIT {`jit`}, hybrid {`jit`, `hybrid`, `jitless`}, jitless {`jitless`}, one filter for every code object (MODES-D13). B2 rule: Guard.policy set in the graph builder; DEOPT narrows, SLOWPATH-capable is a diamond (F3). PIN = a function state that excludes every AotOpt variant, also ones installed later, then select() -> AotBaseline > GenInterp > Interp (C5); final in jitless (H32-Q4), in hybrid until the re-tier policy lifts it (MODES-D14). JIT PROFILED row = any JIT code with a PROFILED input (C1). "Lower confidence" = the C-05 byte (C7). Hybrid part (frozen at K-A4P): `hybrid` lowering, frame -> T0 + counter, speculation exits >= budget -> PIN + re-tier (C4, O2). H32-Q5: the counter unit and the rule (only speculation exits count) are written now; the numbers stay open to the A4 gate (MODES-D12). Plan fix: P02:189 | fill + clarify | Freeze table, parts and the Q5 unit and rule: **A0** (M4-P2): **done** (also Q4 PIN as a function state and the new Q6 with the accept lists). Rule text: **SC**, before batch 2 |
| C-08 | H33 (Draft) | Decision H33:23 "call and exit counters" -> "budget and exit counters" (H23 unit: call + `JumpLoop`). Q1: one RW budget word per code object. Q2-Q4: take the defaults (H33:81-83). "Instrumented T1" = T1 (P05:336). All hybrid-only text goes into the hybrid part (K-A4P). H33-Q5: the counter unit (the H23 budget unit) and the rule are written now; the threshold number stays open to the A4 gate (MODES-D12, O4). Plan fixes: P02:190, P04:124, P05:338 | Draft edit (fill + phrase) | Freeze table and the Q5 unit and rule: **A0** (M4-P2): **done** (also Q1 and Q4). Text (the Decision "call" -> "budget" counters): **SC**, before batch 2 |
| C-09 | H34 (Draft) | The kind list (H34:23) becomes an append-only part: "a new kind that both finalizers handle or reject is a fill", as H20 part 2 (H20:89). Q5 default. Plan fix: P02:191 | Draft edit (fill) | **A0** (H34-Q5 Freeze table): **done** (M4-P2) |
| C-10 | H27 (Accepted) | The native entry, tier manager and deoptimizer build without `jit`; `jit` adds producers only, the regexp JIT included. Whether producers may run is the process permit (L-P), set once before the first isolate; workers inherit it | clarify (first sentence) + fill (permit row; the reviewer confirms the kind) | **B3** (every other row, plan v9 §10) |
| C-12 | H02 (Accepted) + plan | The CompiledUnit hint section stays in ABI 5; v1 writers leave it empty (flag bit 17 = 0). Plan fixes: P04:227, P05:229, P05:349 ("from the unit's facts section"); P02:129 hybrid row (C1); P04:119 (C4 wording) | clarify | **B1** (H2, plan v9 §10); plan fixes to the plan owner (AR:44) |
| C-13 | AR §3.4 table | Add the gated-part and hybrid-part rows of 5.4, and name the append-only lists | process (uses §3.4) | **done M4-P5** (the rows AR:94-106; the orchestrator's Approval AR:112-114): DOCS wrote them, the orchestrator approved, as it wrote none of them (AR:68; MODES-D1; plan v9 §7 step 1; MC-w17-2) |
| C-14 | plan | New `docs/plan/13-execution-modes.md`: this design as the plan home, with the MODES-CHECK-w13 result folded in. It cites the ADR rows. ADR-A0 may cite it, because it exists before M4-P2 | plan | **done in M4-P1** by the P1 docs agent (MODES-D9; it replaces MODES-D4); the P2 integration fixer refreshes its cites after ADR-A0 |
| C-15 | tests | `select()` as a pure function in `ep9_vm` with no caller yet, plus table tests for T1-T11 with fake code objects; negative tests for each append-only list; selector tests for JIT and hybrid tier-up (a `jit` Baseline or Opt object is selected under the JIT and hybrid presets and skipped under jitless) and for PIN with two Active AotOpt variants (neither is selected after PIN, nor an AotOpt object installed after it; MODES-D13, MODES-D14) | test | month 6, before batches 1-2 (MODES-D6: the tests fix the rule before the freeze) |
| C-22 | H24 (Accepted), H14, H31 | (1) R2 and the H14 positions paragraph (H14:73): in an optimized frame the offset slot holds a bytecode offset or a position id that the code object maps to an inline chain (F11). (2) H24-Q3 fill: the debug copy is an Interp variant in the code set; `select()` reads the session (F7). (3) Per kind: InspectorSession is an InvalidatingFuse for JIT code and a GuardFuse at polls for AOT code (H24:22) | (1) rewrite (pre-freeze churn; data: H29:30 needs an inline chain, H24:29 has one offset per frame); (2) fill; (3) clarify of H24:22 + H31 Draft edit | **B3** for H24 and H14 text (H14 in **B1**); H31 part via **FR0** |
| C-23 | H20 part 2 (text only now) | The shape of `Isolate::new_with(IsolateConfig, ExecOptions)` and an opaque `ExecOptions` (section 1.2). No `IsolateConfig` change. The code landed in M5-P0 as a part-2 addition, with the setter for `type feedback: full | ics_only` (MODES-D11) | fill (addition): the H20 part-2 row of 2026-10-28 | Done in M5-P0 (P0-VM5, 3a74cfa); the feedback mode takes effect in M5-P3 (PROFILE6) |
| C-24 | H17 (Draft), H16 (Accepted) | Cell kind ids (H17) and IC handler kinds (H16) become append-only lists in the H20 part-2 form (5.4); the `kinds.rs` module doc lost "may change" in M5-P0 (`crates/ep9_vm/src/kinds.rs:40-47`). Reserve a range for the callable kinds so that `is_callable` is one compare, or record why not (`crates/ep9_vm/src/kinds.rs:207-210` has 3 compares since kind 28; MC-w13-22) | H17: Draft edit; H16: fill (a Freeze-table part) | H17: the M1a step (month 5); H16: **B1** |
| C-25 | H16 (Accepted) | The full H16 Freeze list, in four parts; B1 copies it into the H16 Freeze table (MC-w16-9, MC-w17-4). (1) **Tuning part, a gated part that freezes at the A4 gate** (5.4, MC-w13-15): the T0 recording policy (the implicit cases, `CALL_WINDOW`, the sampled kinds), the keyed cold-miss rule (`crates/ep9_vm/src/keyed_ic.rs:31-43`) and the InstanceOf record rule (a run that throws records nothing, `crates/ep9_vm/src/instance_ic.rs:41-43`) (MC-w15-11), the stub cache size (1024), hash and probe place (the miss path; compiled code through `Generic`) (MC-w15-7a), and the instanceof walk group size (8, `WALK_STEPS`, `crates/ep9_vm/src/instance_ic.rs:215`; MC-w16-5). These values sit today in the H16-Q2 and H16-Q3 cells (H16:133, H16:134), which freeze at M1 exit. The H23 copy of the group note (H23:33) is the B3 writer's: H23 has no Freeze table, so it freezes whole at M1 exit. (2) **Safety rules, frozen with H16 at M1 exit:** no stub cache entry outlives a cell it names; today the table is emptied at every collection, minor ones included (the Codex D2-1 note in H16:134, which replaced "at least at every major GC"). GC5 measures the flush against a minor-GC weak filter in month 5; if the filter wins, GC6 writes it as an H16-Q3 row (plan v10 M5-PLAN-D18). And: at most one group of walk steps goes uncharged, and a walk that uses up the budget misses after at most one group past it. (3) **Append-only lists, no close point** (5.4 Kind A): the stub cache operations (load, store, define), the CacheIR ops, the `GuardType` type set and the IC handler kinds (C-24). (4) **Fact meanings** (C-06), frozen with H16 at M1 exit: the MODES-D16 overflow record, the MODES-D17 "ran" sites and the MODES-D18 rule; the "ran" bit itself comes with the one month-6 schema bump | fill (Freeze-table parts, before M1 exit) | **B1** (H16 parts); **B3** (the H23 part) |
| C-26 | H25 (Accepted) | Inventory rows for the FEEDBACK Call key half (a FunctionInfo native word, copied, so the `native_of(` grep cannot see it) and the MODULE `UNIT` word; the Rules grep check also lists test files; the H25-Q2 lint must also catch copied native words (MC-w13-1). Keeps the D6 sandbox option (section 9) | rewrite (pre-freeze churn; the Rules check now accepts `module.rs:419`; data: the grep); the lint at M1a | rows: **done in M4-P1** (P1 docs agent); lint: the M1a records role |
| C-27 | H16 (Accepted) | Proposed: the H16-Q2 fill (H16:133) and its source line (M09 §2.2 "T0 records every H16 slot kind") get the condition of MODES-D11. T0 runs the `profile.rs` record hooks under `type feedback: full` (JIT and hybrid presets, a training run with `--profile-out`); under `ics_only` (the jitless default) it runs none. The T0 ICs and their IC-side counts (Literal, CloneObject, Construct) stay in both. **D19 clause (MODES-D19, DL:3248; MC-w17-3):** `ics_only` drops only the `profile.rs` record hooks; allocation-site element-kind feedback and every other object-model input stay, or arrays would get other kinds in jitless than in JIT (M-R9) | fill (a condition for a new preset; every existing check keeps its result under `full`; the reviewer confirms the kind) | **B1** (H16 freezes in batch 1); the M09 line to the plan owner (AR:44) |

C-11 (H07) moved to section 7.2.

Cost (est.): C-01 to C-14, C-22 and C-24 to C-27 are text, about 0.25-1 agent-day each, about 6-8 agent-days in total, spread over the roles above. C-15 is 2-4 agent-days.

### 7.2 At A2 start (after M1 exit, before any unit exists; H35 freezes at the A3 gate)

| ID | ADR / code | Change | Kind | Cost (est.) |
|---|---|---|---|---|
| C-11 | H07 (Frozen by then) | Decide unit placement with SIZE-REPORT data: keep "units in the one code range" (H07:52) while they fit; a unit placed outside has no direct call out and JIT code calls it only indirectly. Only if data shows the 128 MiB range is too small | amendment with data, or none | 0.5 day |
| C-16 | H35 S2/S6 | Container v2 (5.3): flags, extension sections, `facts` section, reserved `deps` and `snapshot binding`; rodata "safepoint maps" clarify | rewrite before freeze (container bump) | 1-3 days |
| C-17 | H35 S2 | Function-table entry = function id + digest + variants {kind, lowering, variant tag, entry, meta, CPU bits, fuse refs, osr entries (later), resume map (later)}. An entry may have no native variant: the function then runs its bytecode. The K-A3 SIZE-REPORT compares three coverage policies: all reachable functions, a static heuristic, the PGO set (MC-w15-20, MC-w16-13: GraalVM Native Image keeps cold methods as bytecode, 20% smaller binaries for a 4% average slowdown, MPLR 2026) | fill | 1 day |
| C-18 | H35 S6; `versions.rs`; `interp/mod.rs` | `VM_LAYOUT_VERSION` over the baked layout facts that no other version owns: the `Cx`, FunctionInfo (`FI_FEEDBACK` included) and RuntimeTable offsets and the table length (MC-w13-14; 5.2). Each exported constant names its owning version (OBJECT_LAYOUT, IC_LAYOUT, Value or VM_LAYOUT), and one fact has one owner (7.3; MC-w15-5); `runtime_table_len`, root-shape table hash, codegen flag hash; 3 `Cx` native fields unconditional. M5-P0 P0-VM5 built the version (1), the ungated export `ep9_vm::layout` and the unconditional fields (DL:2753); the header field is open (A2) | rewrite before freeze + code | 1 day |
| C-19 | H35, H27 | Build-id rule of 5.2 (lands with C-18) | rewrite before freeze | 0.5 day |
| C-20 | H35-Q9 | Unit-data slot rule (section 9) | fill | 1-2 days with a measure |
| C-21 | H16-Q5b (A3) | `.ep9prof` = FactSnapshots per unit hash + L2 tables + run count + merge rule; REBASE in `ep9_aot`. The REBASE digest covers the function's own bytecode and text, with positions relative to the function start (MC-w15-16, MC-w16-13): a function record keeps absolute source offsets in fields of its own (`source_start`, `source_end`, `crates/ep9_bytecode/src/unit.rs:861-863`; `positions`, `crates/ep9_bytecode/src/unit.rs:878`), so a writer that hashed the whole record would drop the facts of every function below a one-line edit at the top of the file, against F15. A `--profile-out` run that loaded a unit copies the unit's PROFILED records of functions with no RUNTIME record, marked "carried", run count not bumped (O1: Leyden iterative training; a run on optimized code gives no type facts for its hottest functions). Counts merge per unit only (F-R7). A fact that names a function in another unit (the exporter can name one: a call or instanceof target in any loaded unit, `crates/ep9_vm/src/facts_export.rs:464-482`, `crates/ep9_vm/src/facts_export.rs:578-587`) maps through that unit's digest map, or is dropped (MC-w14-10) | fill | with A3 |

### 7.3 New ADR or not: no H38

Decision: **no new H row.** Every part of this design fits an existing row. `docs/plan/13-execution-modes.md` is the plan home, as 04 is for tiers.

Reasons:

1. The plan defines modes by fact sources and guard policy (P02:6). Those already have owners: H30 and H32. The slot and the registry are H14. Deopt is H29. Fuses are H31. Feedback is H33. Units are H35. Features are H27. Positions are H24.
2. AR §3.3 tests a change per owner of the bytes that code or files depend on (AR:76). A new row that restates H14 or H32 values would create two owners for one value. Then one change would be two Change-log lines, and their kinds could disagree.
3. The embedder surface of the modes is `Isolate::new_with` and the opaque `ExecOptions`. H20 already governs that surface: new lines are part-2 additions, and existing lines never change (golden:3-5; H20:89). `ExecOptions` has no public fields, so no preset value or rank enters the frozen surface. The meaning of each preset stays in H30-H33.
4. AR:3 ties the ADR set to the 37 rows of 02 §6 (P02:8). A new row means a plan change, a new ADR, a review record and a freeze inside months 4-6, which are full (DL:2750).

Reopen: if embedders must get a promise about what a preset does (rank, thresholds) beyond the H32/H33 rows, or if a per-function mode choice is exposed to users, write H38 "Execution modes and code objects" before A2 start.

---

## 8. Plan v9 impact

Plan v9 is kept outside the repo (DL:2743). Its decisions are logged at DL:2747-2832. Month 4 has 6 phases and 35 role slots, with 9 agents or fewer per workflow (DL:2750). This design adds **no role and no agent**. It hands text to roles that already own the rows.

| When | Change | Effect on work in flight |
|---|---|---|
| M4-P1 (done) | PROFILE built T0 type feedback and kept FACT_SCHEMA 1. SPIKEC-1 kept the JIT-mode guard policy and the Opt slot marker (SCP:26, SCP:28). Added after the design: the weekly MODES-CHECK (plan v9 rule 29, MODES-D9); C-14 (this file in the repo) and C-26 (H25 rows) by the P1 docs agent; the stale count doc lines fixed (F-R7) | none |
| M4-P2 ADR-A0 (done) | Input file (this design, sections 1.4, 3.5, 3.6, 4.3, 5.4, 7.1). ADR-A0 writes the H29-H34 Freeze tables with the append-only lists, gated parts and hybrid part (C-01 Q1 default, C-04, C-07, C-08, C-09 tables); H30-Q1, Q2 and Q4 Open-question text (C-05); H17 rep/const proposal and the validity-row sentence (C-02; the H17 validity row needs an Owns change in plan v9 §4.1, see C-02); the ProtoRef validity descriptor kind (C-01); the H32-Q5/H33-Q5 counter unit and rule (MODES-D12); the exit class bit (C-04). It cites this file, which exists since M4-P1 | about +1-1.5 agent-days of text on one existing agent (est.) |
| M4-P2 FACTS2 (done) | F-R7 (count units), F-R8 (Call keys never written out; a shared call/construct slot test), F-R9 (implicit int32 meaning; the cold vs int32 share measure, MODES-D10), the periodic call pattern test (MC-w13-17): built and in the exporter module doc; the H16 text came in the M4-P2 Codex-fix step (C-06, P2-ORCH-12). The measure gave MODES-D15 (F-R9). The H30 Freeze table says C5c is not a jitless number (MC-w13-23) | inside the FACTS2 task (plan v9 §4.1 task 9 hands it these notes) |
| M4-P2 fixer, m4-p2 tag | refreshes the cites of this file at the P2 merge head (done: section 13 item 22); MODES-CHECK at the tag | none |
| M4-P3 | KEYED owns the C-18 layout export (inside `native.rs`, behind `spike-native`, `FI_FEEDBACK` included) and the keyed RuntimeTable entries (P2-ORCH-6, MC-w14-7): done in 8140b68; the move out of the `spike-native` gate stays with C-18 at M5-P0, and the Spike C switch to the export goes to SPIKEC-4 (P3-ORCH-9). IC2's H16-Q3 fill fits H33-Q3 ("FeedbackVector is the store", H33:82). The P3 integration fixer writes the MODES-CHECK-w14 text items (MC-w14-2 to -5, -10, -14, -18, -19, -22) and MODES-D17/D18 (F-R5, F-R9) | none |
| M4-P4 | SPIKEC-4: `vm_layout.rs` reads the C-18 layout export (`ep9_vm::layout`, `FI_FEEDBACK` included, each constant naming its owning version), and keyed sites use RuntimeTable 8 and 9 (`get_keyed`, `set_keyed`) as the generic path (P3-ORCH-9, MC-w15-2, -5); it reports the 4 B call-form exit stub (MC-w15-14). The integration fixer writes the MODES-CHECK-w15 text items (MC-w15-1 to -4, -7a, -10a, and the doc 13 lines of -5, -6, -15) and fixes `native.rs` (OBJECT_LAYOUT 4) | none |
| M4-P5 (done) | DOCS: C-13 (AR §3.4 rows, MODES-D1): done (AR:94-106); the orchestrator approved it (AR:112-114), as it wrote none of the rows (AR:68; plan v9 §7 step 1). The C-23 shape is already decided (MODES-D7) | none |
| Month 5 (plan v10: phases M5-P0 to M5-P5; M5-P5 = M1a) | **M5-P0:** P0-VM5 makes the 3 `Cx` native fields unconditional, adds `VM_LAYOUT_VERSION` 1 and the layout export out of `spike-native` (C-18), removes `Cx.proto_epoch` (DL:2753), and lands C-23 (`ExecOptions`, `Isolate::new_with`, `build_info`) with OBJECT_LAYOUT 5 and IC_LAYOUT 3; DOC13 refreshes this file (MC-w17-2 to -4, MC-w16-9, -13, -20; w16 O2, O3; w17 O3). **M5-P1:** fuse registry v0 (FR0) builds FuseRef with Dynamic and fills the H31:90 rule row before M1a (C-01); PROFILE5 measures the size of a second T0 handler table in the MODES-D19 form, next to the W-PROFILE-1b row, and adds the month-5 jitter of the call sampling (FACTS2); ELEM1 builds element kinds step 1. **M5-P2:** CONSTREP adopts C-02; ELEM2 writes the M-R9 test (w16 O2). **M5-P3:** PROFILE6 builds the `ics_only` table (MODES-D11, D19) and the jitless T0 measure of the no-`jit` build; M1A-CODE writes C-24 for H17 (decided in plan v10: no renumbering) and the H25-Q2 lint that catches copied native words (C-26), with the D6 rules (section 9). **M5-P5 = M1a:** the H17-H19 M1a parts freeze. The month-5 carry rows are plan v10 §11 (MC-w13-12) | < 1 day of text (est.); the second table is code, cost after the size measure |
| Month 6 | SC fills H29-Q1/Q2/Q4 from data, H30/H32/H33 rule text (C-04, C-05 rule text, C-07, C-08). The one FACT_SCHEMA bump (DL:2756) adds no VERIFIED field. C-15 model tests (2-4 agent-days, est.). The bump also adds the sampled "ran" bit (F-R9, MODES-D15). FREEZE-M1: batch 1 takes C-03, C-06, C-12, C-22 (H14 part), C-24 (H16), C-25, C-27; batch 2 the H29-H34 fills; batch 3 C-10, C-22 (H24 part) (plan v9 §10, FREEZE-M1; MODES-D5) | +2-4 agent-days; batch writers carry +0.5-1 day each (est.) |
| Month 7 | None. If C-15 slips, it moves here | buffer only |
| A2 start | C-11 (decide with data), C-16 to C-20 (5-8 agent-days, est.). Text: a fact-validation test switch in `ep9c` and T2 that plants one false fact per producer (PROVEN, REBASE, HINTED); a planted fact must be caught or cost speed only, never output (w14 O1: JSC `--aot-validate`; MC-w15-15) | before any unit exists |
| A3 | C-21; H35 freezes; the fact-validation tests (w14 O1) | |
| A4 gates (M8 exit) | Budget and threshold numbers; the T0 recording policy part; gated H29 part freezes; a jitless-lowering measure (MC-w13-23); K-A4P decides the hybrid part. Each A4 gate kernel also runs inside a large program, not only alone (a large-program column; w14 O2: Bun's median ratio went from 0.94 alone to 1.44 inside a large program; MC-w15-15) | |
| End of A5 | VERIFIED region table and A5v fuses freeze, if built | |

Total now (est.): about 6-8 agent-days of ADR and plan text across A0, FR0, SC and the batch writers, 2-4 agent-days of tests in month 6, and under 1 day of optional code in M5-P0.

---

## 9. Not decided now

| Item | Why not now | Must be decided by | Decider |
|---|---|---|---|
| Ship the `aot` writer in the runtime (F21) | `ep9c` is "not shipped" (P05:427); no data | A4 (before hybrid ships) | product call, logged in DL |
| Closed-world bundle mode | Changes what PROVEN may assume across modules | M7 exit (D17 rule) | product call |
| App heap snapshots before v1 (F19) | Today after v1 (P03:187) | A3 (snapshot binding format) | product call |
| D6 sandbox for untrusted units | D6 = no sandbox in v1 (H25:16). M4-P1 made the H25 inventory incomplete: the FEEDBACK Call key half holds a copied FunctionInfo native word, and the MODULE `UNIT` word (since M3-P2) had no row (MC-w13-1; rows added in M4-P1, C-26). Cheap room to keep (O5: V8 split its code cache into trusted and untrusted parts in October 2026, commit ffac665d76): (1) an untrusted unit may use the BYTECODE path only; (2) every index read from a unit (constant pool, root, link table) is bounds-checked; (3) identity checks run before any merge work; (4) an untrusted unit runs from SOURCE only (w14 O7: QuickJS-ng SECURITY.md, 2026-10-11). Input for option (1): a bounds-only bytecode verifier is enough for BYTECODE-only (w15 O10: V8's sandbox bytecode verifier checks memory safety only and is small, 268c69b002, 2026-10-16) (MC-w15-15) | M1a is the last cheap point (H25:16) | product call |
| JIT warm start from a profile (F17) | No data; K-A4H and K-A4P decide hint and hybrid value (P05:379-380) | M8 exit | AOT role with data |
| Rank order of each preset beyond 1.2 | Needs T2 and A4 data | A4 gate | AOT role |
| Budget and threshold numbers (4/site, 16/function, 1,000 calls) | Tuning values with later freeze points (H32-Q5, H33-Q5). The counter unit and the rule are not open: ADR-A0 writes them in M4-P2 (MODES-D12). JSC scales its thresholds by core count (O4), so the A4 call also decides whether the numbers become ExecPolicy defaults outside G8 | A4 gate | AOT role; product call for the G8 question |
| When the hybrid re-tier lifts PIN (MODES-D14) | Part of the hybrid re-tier policy; no data before A4. In jitless PIN never lifts (H32-Q4) | K-A4P (the hybrid part, 5.4 kind C) | AOT role |
| Unit-data slot rule (64 slots fixed at build, H35:155) | Options: slot 0 for the engine unit + `ep9c`-assigned slots; a map-time slot word in an RW page (one more load per entry, est.); or one exported header symbol per unit, resolved by the embedder, slot assigned at load (O6: the .NET 11 platform-native ReadyToRun form). Unit unloading must also handle Call keys: a feedback vector can name another unit's FunctionInfo, whatever that unit's reachability; `Runtime.units` and `Runtime.infos` are never freed today (`crates/ep9_vm/src/runtime.rs:246`, `crates/ep9_vm/src/runtime.rs:251`; F-R8, F20) | A2 | AOT role with a measure |
| Unit placement outside the code range (C-11) | Gives away the H07:58 gain; no size data yet | A2 (SIZE-REPORT) | JIT/platform role with data |
| Deopt record format, location encoding, swap rule (H29-Q1/Q2/Q4) | Spike C and the month-6 probe measure them (SCP:30) | M1 exit (or the M5 part, DL:2778) | SC role |
| Materialize payload format | No escape analysis before M7 | A4 gates (gated part) | JIT/platform role |
| Per-function fallback in the loader | Needs the function table | A3 | AOT role |
| Relax build-id step 3 to "same ABI -> BYTECODE" | Only when deployments need it (H35:162) | when needed | AOT role |
| H38 | See 7.3 | before A2 start, only if the reopen condition holds | orchestrator |
| Second T0 handler table vs a flag test in each hook (MODES-D11 fallback) | Needs a SIZE-REPORT measure of the extra table | month 5, before the build | orchestrator with SIZE-REPORT data |
| A callable kind range (MC-w13-22) | Kind ids are not yet an append-only list (C-24). **Decided in plan v10 (M5-PLAN-D5):** no renumbering; `is_callable` keeps its 3 compares, and the reason goes into the `kinds.rs` module doc (P0-VM5, M5-P0) and the H17 text (M1A-CODE, M5-P3) | M1a | M1a step |

---

## 10. Risks

| # | Risk | Effect | Mitigation |
|---|---|---|---|
| R1 | Reserved values freeze on paper with no test. The pre-freeze gates test only the JIT row (SCP:28, SCP:128) | A reserved format is wrong when built | Reserve only list values, never formats. Formats sit in gated parts that freeze at the end of their build. C-15 tests the state machine with no codegen |
| R2 | Append-only lists used to dodge G8 | Rewrites hide as appends | An append only adds a value; an existing value never changes (H20:89 form). Negative tests probe the top of the range, so an append keeps every result. The reviewer confirms the kind (AR:79) |
| R3 | Month 5-6 slips | Rows freeze without the fills | Text items are small and spread over A0, FR0, SC and the batches. C-01 has the earliest deadline (M1a); A0 writes its Q1 default in M4-P2. C-15 can move to month 7 |
| R4 | REBASE matches the wrong function | Stale facts reach the compiler | Digest = bytecode body + source-text hash. Facts stay guarded (H30:60). The worst case is speed, never output (P04:325) |
| R5 | Hybrid deopt storm while T1 compiles | Slow warm-up | PIN at the function exit budget (T4) |
| R6 | After C-19, builds with different native layouts share a build id | Native code reads wrong `Cx` offsets | C-18 lands with C-19; `--unit-strict` in CI |
| R7 | Lowering class adds a test axis | More A3/A4 test cost | `jitless` lowering runs everywhere. The mode x tier differential (P04:324) adds one axis value, not a suite (est.) |
| R8 | 128 MiB code range shared by JIT code and unit text (H07:53) | Many units plus hybrid JIT exceed it | SIZE-REPORT tracks size (P05:429); C-11 decides at A2 with data |
| R11 | Jitless isolates pay T0 record costs until the `ics_only` table exists | a64 navier-stokes +9.39%, richards +6.05% for records nobody reads (W-PROFILE-1b share) | MODES-D11: the second table in month 5 after the size measure; fallback flag test only if ≤ +0.5% in JIT mode |
| R12 | An empty BinaryOp or CompareOp slot is "int32 or never ran" (F-R9) | AOT guards int32 at sites that never ran in training; each later non-int32 value spends site budget, then the site goes SLOWPATH | H32 site budgets bound the cost; FACTS2 measured the share (10-15% of empty sites are cold); the sampled "ran" bit goes into the month-6 schema bump (MODES-D15) |

R9 (the AR §3.4 reading is refused) and R10 (the C-22 R2 rewrite is refused) are closed: MODES-D1 and MODES-D2 accepted both. Their fallbacks are in section 11.

---

## 11. Rejected alternatives

| Alternative | Why rejected |
|---|---|
| A global `Mode` enum read by `ep9_vm` and the compilers | Spreads mode branches. The plan wants them only in fact sources and guard policy (P04:237). Three layers cover every case, the jitless preset on a JIT build included |
| `ExecPolicy` or `TierHooks` as new `IsolateConfig` fields | `IsolateConfig` is a core line of the Frozen H20 core (golden:467). A changed line is a rewrite of a Frozen part (golden:3-5; H20:88). A new entry point is an addition |
| Guard lowering after mode-blind passes | A SLOWPATH guard rejoins the main path (H32:39-43); blind passes would assume facts it breaks. H30:65 and H32:61 put the policy in the IR |
| A separate AOT compiler, a second IR or an LLVM path | One emitter, two finalizers (H34:23-30). JEP 295 `jaotc` was removed for low use and high upkeep. H34 says no LLVM |
| An AOT-only runtime without T0 | H5 forbids it (P02:162). Dart and GraalVM had to add interpreters back |
| Deopt-only guards in jitless code | Forever deopt (P05:311); H32 keeps SLOWPATH and PIN |
| A second word or a list in the dispatch slot | Moves the `FI_*` offsets that native code bakes in (`crates/ep9_vm/src/layout.rs:185-191`) and costs every call. The cold code set gives the same room |
| Reserving slot bytes 1-7 now | They are Rust padding (`crates/ep9_vm/src/dispatch.rs:77-80`); reserving them needs code, and a later field there moves no offset |
| Extension parts frozen at the start of their stage | The value is then turned on after its freeze, a G8 rewrite (AR:76, AR:84). Append-only lists and end-of-build freezes avoid it |
| VERIFIED as FactSnapshot records with source byte 3 | FactSnapshot is keyed by (function, slot) (H16:82); VERIFIED by region version (H30:96). It would be a format change after the H16 freeze (H16:97) |
| An extension-block trailer inside FactSnapshot | Weakens "one encoding per fact set" (H16:99) and has no merge rule for unknown blocks. L2 tables in the container give the same room |
| A `facts_key` hash that skips the hint section | A second hash rule in H16 `validate`. F-R1 makes it unneeded: no fact lives in the CompiledUnit |
| A per-function source hash inside FactSnapshot | FactSnapshot is strict by design (H16:100). REBASE does the job before it |
| A new dispatch kind for "instrumented T1" | T1 already writes the full FeedbackVector (P05:336) |
| Kind bytes for region code, CPU variants or the debug copy | Variants live in the code set; kind bytes stay few |
| Full or sampled feedback from optimized-AOT code | Size and speed cost; .NET never instruments R2R code (P04:122). The hybrid re-tier through T1 gives fresh feedback |
| Building OSR into AOT, the resume map or the re-tier now | Each is gated (A4, K-A4P). A4 costs 16-32 PM, the re-tier alone 2-4 PM (est., P05:405). Only list values are needed now |
| Unit text outside the code range now (old C-11) | Gives away H07:58 (direct calls between JIT code, units and stubs). The evidence (H34:91) is today's SP-Unit scan, not a contract. Decide at A2 with data |
| A new docs role MODES in M4-P4 | ADR-A0 already owns the H29-H34 Freeze-table and Open-question rows and the H17 rows (plan v9 §4.1); two roles on one row collide |
| Dropping `VM_LAYOUT_VERSION` | C-19 removes features from the build id, so the id alone no longer pins the `Cx` layout. Since M5-P0 the `Cx` layout no longer depends on the features (`crates/ep9_vm/src/interp/mod.rs:163-177`), but the `Cx`, FunctionInfo and RuntimeTable offsets still change between engine builds, and `VM_LAYOUT_VERSION` is their one owner (`crates/ep9_vm/src/versions.rs:60-69`) |
| A new H38 row now | See 7.3 |
| "An implicit bit never counts as seen" (MC-w13-6) | T0 never writes INT32 at BinaryOp or CompareOp sites, so there is no seen-INT32 to keep apart; every int32 fact would go low-confidence and AOT would lose int32 everywhere |
| A "ran" bit on every hooked site (MC-w13-6 option B) | Costs T0 instructions on every hooked op, against V9-D6 |
| `feedback_threshold = u32::MAX` as the jitless "no records" switch | It makes no feedback vector, so it also turns off every IC (`crates/ep9_vm/src/interp/mod.rs:147-149`; `crates/ep9_vm/src/feedback.rs:174-179`) |
| One flag test in each record hook as the first choice (MC-w13-10 option C) | Costs instructions in JIT mode too; kept only as the MODES-D11 fallback |
| T2 and optimized-AOT v1 inline only callees that make no runtime call and no poll (the old C-22 fallback, R10) | Costs T2 and AOT peak speed for the life of v1. Every production engine with inlining maps positions to an inline chain. MODES-D2 accepted the C-22 (1) rewrite instead |
| A new AR §3.3 clause for parts built later (the old C-13 fallback, R9) | A process change that needs a second reviewer from another role (AR:65, AR:68). AR:84 and the H20 part-2 form (H20:89) already cover it (MODES-D1) |

---

## 12. Review log

Source: the adversarial critique of the first version (23 findings; the text is kept outside the repo). Each (a)/(b) finding was checked against fc6ac78 before acting. Sort: (a) serves the goal; (b) a defect in this design; (c) neither.

| Finding | Sort | Verified | Result | Why / what changed |
|---|---|---|---|---|
| F1 `ExecPolicy`/`TierHooks` in `IsolateConfig` | (b) | yes: golden:334 is a core `embed:` line; golden:3-5; H20:88 lists `IsolateConfig` and its pub fields as core; 8 fields at `isolate.rs:51-73` | **applied** | 1.1 "Today" cell fixed; 1.2 new entry point + opaque `ExecOptions` (part-2 additions); S-2 moved; 7.3 item 3 rewritten; C-23; rejected-alternative row |
| F2 extension parts freeze at stage start | (a)+(b) | yes: AR:76, AR:84; A5 after M7 (M09:62, M09:216) | **applied, modified** | 5.4 rewritten. Main tool = append-only parts with no close point, the existing H20 part-2 form (H20:7, H20:89), not the proposed X0 clause (no AR change needed). Gated parts freeze at the gate that ends the build. "Activation" removed (R2) |
| F3 guards lowered after mode-blind passes | (b) | yes: H32:39-43, H32:61, H30:65, H32:89 | **applied** | B2 = guard creation in the graph builder; passes read `Guard.policy`; P-R1 text, P-R5 added; diagram; C-05, C-07 text |
| F4 H34 kind list "internal" | (a) | yes: H34:23 Decision; P02:191; AR:76 | **applied** | C-09 = append-only part (H34 is Draft) |
| F5 C-11 gives away H07:58 | (a) | yes: H07:58; H34:91 is an SP-Unit scan; AR:65 | **applied** | C-11 deferred to A2 with SIZE-REPORT data (7.2); F24, R8, section 9 |
| F6 VERIFIED byte 3 reserves a name only | (b) | yes: H16:82; `snapshot.rs:63-71`; H30:96; H16:97 | **applied** | own region table (4.1, F-R5, 4.4, C-05) |
| F7 `select()` ignores the debugger | (b) | yes: H24:75 (Q3), H24:22, H31:40 | **applied** | debug copy = Interp variant; `select()` reads the session; T10 per code kind; T11; C-22 (2), (3) |
| F8 role collision with plan v9 | (b) | yes: plan v9 §4.1 (ADR-A0 Owns and tasks 2, 3, 5); plan v9 §10; SCP:30; DL:2778 | **applied** | no MODES role; routing to A0, FR0, SC, B1-B3 (7.1, 8); C-04 reserves tag values only; no forward cite (C-14 after A0) |
| F9 hybrid rows freeze before K-A4P | (a) | yes: P05:380; H33:88; H33:84 | **applied** | hybrid part frozen at K-A4P (5.4 kind C; C-07, C-08; F2 row) |
| F10 false reason for `VM_LAYOUT_VERSION` | (b) | yes: H35:80, H35:82, H35:162; P05:283 | **applied** | 5.2 reason = C-19 + `Cx` features; R6; rejected-alternative row |
| F11 no "call" record tag; inline positions | (a) | yes: H29:23, H29:37; H24:29; H14:73; H29:30 | **applied** | tag "call" in v1 (3.6, C-04); position-id rule C-22 (1) with a fallback (R10). Kind rewrite of an Accepted row: orchestrator decision |
| F12 `produce` misses regexp JIT | (b) | yes: H27:9, H27:29; the prior-art survey | **applied** | 1.2 `produce` lists every producer; jitless empties all; C-10 |
| F13 security is per process | (a) | yes: H07:52; H22:40; P08:117 | **applied** | layer L-P permit; M-R8 workers; `forms {o, ep9u}`; C-10 |
| F14 bytecode life | (b) | yes: H04:80; H35:111 | **applied** | M-R7 |
| F15 realms vs code set | (b) | yes: H31:83; H22:23, H22:87; `ops.rs:330-333` | **applied** | 3.5 realm rule; `select()` deps per isolate; C-01 |
| F16 F21 contradicts "`jit` never in a unit" | (b) | yes: design 1.4; H34:37-38 | **applied** | F21 renamed "re-finalize a JIT request as a `hybrid` unit" |
| F17 `CodeDesc` lacks unwind/positions/perf | (b) | yes: H14:92; P02:185; H24:48, H24:74; P03:53; H35:35 | **applied** | CodeDesc fields; stack maps "empty in v1, reserved"; H35:35 clarify in C-16 |
| F18 wrong change kinds | (b) | yes: `dispatch.rs:77-80` padding; H14:22, H14:69; AR:74-76; P02:189/190; P04:119/124; P05:338 | **applied** | C-03 = rewrite (pre-freeze, with data); slot bytes not reserved; plan fixes added to C-07, C-08, C-09, C-12 |
| F19 "class S" name clash | (b) | yes: H15:62; DL:2778 | **applied** | renamed to lowering class `jit`/`hybrid`/`jitless` |
| F20 `select()` code with no caller | (c) | n/a | **left** | C-15 unchanged; listed in section 9 for the orchestrator |
| F21 impossible preset | (a) | yes: `isolate.rs:98-100`; P05:282 | **applied** | M-R6: error, reusing `IsolateError::Config` (a new variant would change a Frozen core type) |
| F22 root-shape identity | (a) | yes: H35:23, H35:89; H30:45 | **applied** | root-shape hash in the ladder, 5.2, C-18 |
| F23 M-R3 oracle claim | (b) | yes: P02:27; P02:184 | **applied** | M-R3 = extra test; G9 oracle = no-`jit` build |

---

## 13. Changes since the first version (MODES-CHECK-w13, MODES-D9..D15 and the M4-P2 refresh, 2026-10-27)

Sources: the MODES-CHECK-w13 report (§3 findings, §6 updates 1-10) and its research notes (kept outside the repo), and the orchestrator decisions MODES-D9..D12.

1. Status: decided design (MODES-D1..D12). This file is the repo home of the design, written in M4-P1 (MODES-D9; C-14 moved from "after A0").
2. §0: a summary of what the first weekly check found and the four new rules.
3. §1.2 and C-23: ExecPolicy field `type feedback: full | ics_only` (MODES-D11; w13 update 2). Jitless default `ics_only`; training runs, JIT and hybrid `full`. Second T0 handler table per `Cx`, after a SIZE-REPORT measure; fallback flag test only if ≤ +0.5% in JIT mode. W-PROFILE-1b reason holds for JIT and hybrid only (MC-w13-19). RegExp producer chosen per isolate (MC-w13-16).
4. §1.3: Interp writes ICs only under `ics_only`.
5. §3.3 T4 and §3.6: exit class bit {speculation, environment}; only speculation exits count toward Demote and PIN (O2). The exit record names its feedback slot, as a note (MC-w13-7; update 5).
6. §3.5, C-01, C-02: "prototype-chain validity by ProtoRef" is a Dynamic FuseRef descriptor kind; the descriptor names the prototype, not the cell; the registry keeps the dependents; one sentence in the H17 validity row (MC-w13-11; update 3).
7. §4.1: the L2 tier-hint shape = priority byte, stability tri-state, exit counts per class (O3).
8. §4.2 and C-21: `--profile-out` copies carried PROFILED records for functions with no RUNTIME record (O1); T0 records only under `full`.
9. §4.3: new rules F-R7 (count units; update 4, MC-w13-2), F-R8 (Call keys never written out; update 8, MC-w13-8), F-R9 (implicit int32 = "int32 or never ran", MODES-D10, MC-w13-6). F-R5 gets the keyed bits 4-9 (MC-w13-9) and the "ran" bit candidate. F-R6 covers carried records.
10. §4.4: C5c measures JIT lowering only; A4 needs a jitless-lowering measure (update 9, MC-w13-23).
11. §5.2 and C-18: `FI_FEEDBACK` joins the VM-layout set; baked layout facts move into `ep9_vm::native::layout` (update 1, MC-w13-14).
12. §5.4: append-only cell kind ids (H17) and IC handler kinds (H16) (C-24; MC-w13-13, -22); the T0 recording policy as a part that freezes at the A4 gate (C-25; MC-w13-15); the H32/H33 counter unit and rule written in M4-P2, numbers at A4 (MODES-D12, O4) (update 6).
13. §6: the F-room list has 24 items, F1-F24, and plan v9 rule 29 checks all 24 (update 10). §6.1 lists the window effects (F6, F21 widened; F13 widened data; F16, F18, F20 narrowed; F8 needs text).
14. §7.1: C-01, C-02, C-04, C-06, C-07, C-08, C-14, C-23 updated; new C-24 (append-only kind ids and handler kinds), C-25 (T0 recording policy part), C-26 (H25 rows and lint, MC-w13-1). New route FACTS2 (M4-P2).
15. §8: M4-P1 done; M4-P2 rows for ADR-A0, FACTS2 and the fixer; month 5 adds the `ics_only` table, the jitless T0 measure, the jitter and the M1a items.
16. §9: D6 row cites MC-w13-1 and the 3 options of O5; the unit-data slot row gets the O6 option and the cross-unit Call-key rule (update 7); the budgets row says unit and rule are written now; 3 new open rows (the "ran" bit, the table-size fallback, a callable kind range).
17. §10: risks R11 (jitless pays record cost until `ics_only`) and R12 (implicit int32 cost in AOT).
18. §11: 4 rejected alternatives from the check.
19. Plan v9 line cites became "plan v9 §x" cites, because plan v9 line numbers move.
20. Critic of the M4-P1 commits (2026-10-27): the decided items read as decided (MODES-D1 §5.4 and C-13; MODES-D2 §3.6; MODES-D6 C-15; MODES-D7 §1.2 and C-23); R9 and R10 closed, their fallbacks moved to section 11; one owner (A0) for the H17 validity sentence; the C-05 Open-question text routed to A0 (MODES-D3); C-22 (H14 part) in batch 1 (MODES-D5); new C-27 (H16-Q2 under `ics_only`); §4.2 says IC-side counts stay under `ics_only`; C-26 kind rewrite; DL:2833 -> DL:2832; the §7.1 table is one table again (a blank line split it before C-24).
21. Codex review of this file (M4-P1 round 3, 2026-10-27), 2 findings [medium], both a defect in this design. (1) The `accept` filter rejected runtime JIT code: no preset listed `jit` (MODES-D13). Now JIT accepts {`jit`}, hybrid {`jit`, `hybrid`, `jitless`}, jitless {`jitless`}, one filter for every code object; a unit variant that names `jit` is skipped (§0 diagram, §1.2 table and bullet, §1.4, §3.2, T2, C-07). (2) PIN marked one AotOpt object Demoted, so `select()` could pick another AotOpt variant (MODES-D14). Now PIN is a function state in the code set that excludes every AotOpt variant, also later ones; final in jitless (H32-Q4), in hybrid until the re-tier policy lifts it. The object state Demoted is gone and "Demote" reads PIN (§1.3, §3.1, §3.2, §3.3 diagram, T1, T4, C4, §3.6, §5.4 kind C, F5, C-07, R5). C-15 gets the tier-up tests and the PIN test with two Active AotOpt variants; §9 gets the open hybrid lift rule.
22. M4-P2 refresh (the M4-P2 integration fixer, 2026-10-27; MODES-D9). Cites re-checked at the M4-P2 merge head with the ADRLINE drift script and by reading each moved line. 6 cites moved with the same line text: `spikes/spike-c/src/ir/mod.rs` 278-289 -> 280-291 and 276-289 -> 278-291, `spikes/spike-c/src/emit_a64/mod.rs` 34-40 -> 37-43, `crates/ep9_vm/src/feedback.rs` 173-178 -> 174-179 (twice) and 171 -> 172. 4 cites whose line changed were set by hand: `crates/ep9_regexp/src/lib.rs` 115-130 -> 165-183 (the `Backend` trait after RX0A; the claim holds), `crates/ep9_vm/src/facts_export.rs` 9-13 -> 9-16 (its sentence was false after FACTS2, rewritten in §4.4), and the two `emit_a64/mod.rs` cites of the §5.2 `FI_FEEDBACK` bullet, whose sentence was false after d5a89d5 (the emitter no longer hard-codes 2^49; rewritten). H33:88 -> H33:99 (ADR-A0's Freeze table moved the K-A4P line); every other `Hnn:n` cite into an ADR that M4-P2 changed names the same row, which only got longer. SCP:101, SCP:124 and SCP:125 -> SCP:103, SCP:127 and SCP:128 (the same lines after this fixer's §2.1 note and §3 harness line in SCP). Marked done: the ADR-A0 parts of C-01, C-02, C-04, C-05, C-07, C-08 and C-09 (§7.1) and the M4-P2 rows of §8; MC-w13-23 now sits in the H30 Freeze table (§4.4). MODES-D15 (P2-ORCH-3) is written into F-R5, F-R9, C-06, §0, the month-6 row of §8 and R12, and the §9 row of the "ran" bit is gone (decided). P2-ORCH-6 (the keyed RuntimeTable entries and the `layout` export of the spike's private copies go to M4-P3 KEYED) is written into §5.2. Open: the H16 text of F-R7, F-R8 and F-R9 (C-06) was written by neither FACTS2 nor ADR-A0; the exporter module doc has it.
23. M4-P2 review fixes (the M4-P2 integration fixer, part B, 2026-10-27), from the M4-P2 Claude reviewers. In the contracts this design names: the H16 CacheIR op `GuardValidity(CellRef)` became `GuardProtoValidity(ProtoRef)` (the handler's cell in T0, a Dynamic FuseRef in compiled and unit code: §3.5, C-01, MC-w13-11), and the ADD and lexical-binding rows got the T0 checks they lacked (`GuardSlotsRoom`, `GuardNotHole`); H33's budget unit, re-tier start rule and threshold number moved from the Rule part (M1 exit) into the Hybrid part (K-A4P), as MODES-D1 asks for hybrid-only rows (C-08); H34's JIT finalize forms freeze at M7 exit (T2 builds them last) and its unit finalize forms split into a baseline-AOT part (A3 gate) and an optimized-AOT part (A4 gates) (C-09); H29-Q6 keeps OSR out of AOT code (a lazy deopt at the `JumpLoop` poll) in the Contract and leaves the tier-up to the H33 Hybrid part (T8, C-04); H31-Q1 says no unit code bakes a fuse-word address and JIT code may use the immediate LinkSlot form. The FACTS2 exporter gives no record for a `CreateRegExpLiteral` slot (F-R7 context), and `crates/ep9_vm/src/facts_export.rs` 68-78 -> 69-79 and 68-102 -> 69-103 moved with the same text. Spike C (§4.2, B2): the overflow guard of `+`, `-`, Inc and Dec is unbacked (the profile leaves that overflow implicit, F-R9), and a loop phi's entry check is backed only by a check in the loop header, so am3 reads 11 fact + 18 unbacked guards (was 23 + 6), same code bytes. Open for the orchestrator: H16 GLOBAL stores (Codex r1, the CacheIR map gives GLOBAL a load only).
24. M4-P2 Codex-fix docs step (2026-10-27). F-R9 gets MODES-D16 (P2-ORCH-10: T0 records the int32 `+`, `-`, `Inc` and `Dec` overflow from the month-5 PROFILE step). C-06 gets P2-ORCH-9 (the CacheIR map depends on the operation; StoreGlobalField for a `GLOBAL` store) and its route says the H16 text of F-R7, F-R8 and F-R9 is now written (P2-ORCH-12); the M4-P2 FACTS2 row of §8 says the same.
25. M4-P3 integration fixer, docs part A (2026-10-27; MODES-CHECK-w14 text items, plan v9 §5.8). §5.4 gets two rule lines: an appended value names every frozen list that accepts it (U3, MC-w14-3), and a list needed at M1 that grows later is split into an M1 list and an append-only list (the H30 Q3 fused kinds and the H30 ref-node kinds, MC-w14-4); Kind A gets the H16 `GuardType` type set and the H30 rows (MC-w14-19, -4), Kind B the H34 T3 part (U11, MC-w14-14), and the H32/H33 gated row names the part of each counter unit and rule (U2, MC-w14-2). F-R5 and F-R9 get MODES-D17 (the "ran" bit covers LoadKeyed) and MODES-D18 (only with the keyed receiver, out-of-bounds and hole bits), the `BinaryHint::INT32_OVERFLOW` record of MODES-D16 (MC-w14-19) and the INDEX-only LoadKeyed meaning (U4, MC-w14-6). C-06 names the `GuardType` set; C-21 maps a fact about another unit's function through that unit's digest map, or drops it (U5, MC-w14-10). §5.2 and §8: KEYED made the C-18 layout export and the keyed RuntimeTable entries (U1, MC-w14-2, MC-w14-7); the Spike C switch goes to SPIKEC-4 (P3-ORCH-9). 7 Spike C cites moved with the same line text after SPIKEC-3 (8c0e254, ed620e8): `spikes/spike-c/src/ir/mod.rs` 280-291 -> 294-305 and 278-291 -> 292-305, `spikes/spike-c/src/emit_a64/mod.rs` 507 -> 559, 867-868 -> 919-920, 150-160 -> 195-205, 858-860 -> 910-912 and 37-43 -> 52-58. The `ep9_vm` cites that KEYED and IC2 moved (`native.rs`, `ic.rs`) wait for the ADRLINE run of the P3 fixer, part B, after the code fixes. [Done 2026-10-27: 5eaf080 (fixer part B) moved them, item 27.]
26. M4-P3 Codex round 1 docs fix (the M4-P3 integration fixer C2, docs, 2026-10-27), from the cx-docs finding: MODES-CHECK-w14 added the fast-path fuse table to H31 above its Rules section (e902a53: 9 more lines), so 4 `Hnn:n` cites into H31 (7 places) named other lines (a heading, a table header, a blank line). Moved with the same row text (read at 830a788 and at a958aad): H31:55 -> H31:64 (TS-derived CHA and enum folds, §3.5), H31:57 -> H31:66 (the stable-index rule row: §3.5, the F8 row of §6, C-01, the month-5 row of §8), H31:82 -> H31:91 (H31-Q1, §3.5) and H31:83 -> H31:92 (H31-Q2, §3.5 realms). The ADRLINE drift script does not read the `Hnn:n` form. The §12 history keeps its fc6ac78 lines (F7 H31:40, F15 H31:83). Every other `Hnn:n` and short-path cite of this file into a file that M4-P3 changed names the same line or the same table row as at 830a788 (a script kept outside the repo, then each moved line read).
27. M4-P3 ADRLINE (5eaf080, the M4-P3 integration fixer part B, after its code fixes) moved the `ep9_vm` cites that KEYED and IC2 moved (`native.rs`, `ic.rs`) to the same line text, and the Codex round-1 docs fix (035ff95) moved the H31 shorthand cites (item 26). So the cites matched the M4-P3 merge head (03f6053); the header said M4-P2 until M4-P4 (MODES-CHECK-w15 MC-w15-4).
28. M4-P4 integration fixer A (2026-10-27; MODES-CHECK-w15 text items, plan v9 §6.7): the header and Origin lines (MODES-D1..D18; MC-w15-4); §3.4 the RuntimeTable no-record rule (MC-w15-6); F-R5 and the "What the M1 gate measures" line: the keyed IC state is live only for StoreKeyed (MC-w15-1); C-18: VM_LAYOUT covers only what no other version owns, and each export names its owning version (MC-w15-5); §6.2 future-room notes and the A2, A4 and D6 inputs from the w14 outside items (MC-w15-15); §8 the M4-P4 row (MC-w15-2). Then the ADRLINE run of the M4-P4 merge head moved the cites that the P4 roles moved, each to the same line text. MC-w15-3 (the H34 T3 part split into the JIT forms, frozen at the T3 gate, and the unit forms, frozen at the end of A5) was made in H34 only (H34:106-107); the §5.4 Kind B row stayed one row until item 29 (MODES-CHECK-w16 MC-w16-1).
29. M4-P5 modes fixer (2026-10-28; the doc 13 parts of the MODES-CHECK-w16 text items, plan v9 §7 step 1). §5.4 Kind B: the H34 T3 row is split into the JIT forms (the T3 gate) and the unit forms (the end of A5), as in H34:106-107 (MC-w16-1). Header: the cites name their lines at 4941d6a, the m4-p4 tag (MC-w16-2). The M4-P4 integration fixer B's PROXY2 caller list (f6ef5e7) added 24 lines to H31 above its Rules section, so 4 `Hnn:n` cites into H31 (7 places) named other lines (a PROXY2 row, a blank line, a heading); moved with the same row text (read at 8f30552 and at 4941d6a): H31:66 -> H31:90 (the stable-index rule row: §3.5, the F8 row of §6, C-01, the month-5 row of §8), H31:91 -> H31:115 (H31-Q1, §3.5), H31:64 -> H31:88 (TS-derived CHA and enum folds, §3.5) and H31:92 -> H31:116 (H31-Q2, §3.5 realms) (MC-w16-2). §3.6: Spike C's exit records use their own probe form, not the reserved location tags; the form choices go to SC (MC-w16-3, MC-w16-21). §5.2: the POLY_IC handler half is the one layout fact Spike C does not take from the export; C-18 adds `POLY_HANDLER` at M5-P0 (MC-w16-4). One full-path cite was one line off since b04b891 and is set by hand: `crates/ep9_vm/src/profile.rs` 106-108 -> 106-107 (F-R9, the keyed cold-miss rule; line 108 is the next bullet). Every other `Hnn:n` and short-form cite of §0-§11 names the same line text at 4941d6a as at fc4992a, the head that MODES-CHECK-w16 checked (a script kept outside the repo), and the full-path cites pass the ADRLINE drift check.
30. M4-P5 integration fixer (2026-10-28; plan v9 §7, the GATES-M4 short-cite report). Three `golden:334` cites of §1.1, §1.2 and §11 (`IsolateConfig`, a core `embed:` line) named another line since the golden grew above it after m4-p1: at 4941d6a `pub struct IsolateConfig` is golden:352 and golden:334 is `JsStr::at`. Set to golden:352 (read at 4941d6a and at the M4-P5 merge head); the §12 copy keeps its fc6ac78 line. The short-cite check (`cargo xtask adr-lint`, `tools/xtask/src/exit/cites.rs`) compares each cite between the header base and HEAD, so it cannot see a cite that was wrong at the base; these three were found by reading. C-13 Route and the §8 M4-P5 row: DOCS writes the AR §3.4 rows and the orchestrator approves (plan v9 §7 step 1).
31. M5-P0 DOC13 (2026-10-28; plan v10 §2.9, the doc 13 items due at the m5-p0 tag). Header: the two month-5 cite rules (plan v10 rules 30 and 31; MC-w17-1, MC-w17-8) and MODES-D19 in the Origin line. §1.1: M-R9, no object-model decision reads `ExecPolicy` (w16 outside item O2); §1.2 and C-27: the MODES-D19 clause (MC-w17-3). §3.2: counter sessions (MC-w15-17); §3.6: the exit-reason key (MC-w15-19); §6.2: the F19 per-run state note (MC-w15-17, with the WeakRef `pending` flag), the F9 WeakRef `deref` note (MC-w16-20) and the builtin effect table (w17 outside item O3); C-17 and C-21: MC-w15-20 and MC-w15-16 (these four items are MC-w16-13). §5.4: a retired Kind A value stays reserved (w16 outside item O3); the Kind B H16 row and C-25: the full H16 Freeze list (MC-w16-9, MC-w17-4). C-13 Route cell and the §8 M4-P5 row: done (MC-w17-2); the §8 Month-5 row names the plan v10 phases and drops "the C-13 step"; §9: the callable kind range is decided (M5-PLAN-D5). Every new cite was read at 3c9f696, the M5-P0 base; the cites into files that P0-VM5 changes are checked again after that merge. `Cites at` stays 10b21eb: that is the M4-P5 merge head, the one rule 30 names for M5-P0 (moved in 32d759f, MC-w17-1).

**Cite check (281e628).** A script read every `file:line` cite of the first version (469 cites) at fc6ac78 and at 281e628, and mapped each through the `git diff -U0` hunks (a script kept outside the repo). 361 are in files that did not change, 100 are in changed files at the same line with the same text, and 2 sit in a changed table row whose rule text is the same (H04:52, H23:33). 6 moved: 5 were updated (`interp/ops.rs` 327 -> 390 and 330-333 -> 393-396; `interp/mod.rs` 156-165 -> 159-168 and 156-169 -> 159-172, twice); the 6th is in the section 12 history and keeps its fc6ac78 line. Reading each cite with its claim found 1 wrong cite: P05:283 -> P05:279 ("BYTECODE ... Drop native code"). The 46 new cites were read at 281e628. In this file, code paths are written from the repo root (`crates/...`, `spikes/...`), so the ADRLINE drift script tracks them.
