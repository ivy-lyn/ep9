# H16: ICs and feedback; feedback schema = FactSnapshot schema

| Field | Value |
|---|---|
| Status | Draft (2026-10-21): slot kinds H16 v0 and FactSnapshot v0 built (`ep9_bytecode` docs, `ep9_facts`). The FeedbackVector and IC v0 are week-4 P2/P3 work |
| Fix by | Slot kinds M0 wk4; frozen M1 exit |
| Freeze point | M1 exit |
| Origin | v1#12 + R16-D7 + R17; plan status E |
| Crates | `ep9_bytecode` (slot kinds, `feedback.rs`), `ep9_facts` (FactSnapshot schema), `ep9_vm` (FeedbackVector, ICs), `ep9_unit` (fact-schema version); later `ep9_baseline`, `ep9_opt`, `ep9_aot` |
| Owner (plan role) | VM lead (ICs, feedback); AOT senior (slot kinds as the FactSnapshot schema) |
| Validated by | `ep9_bytecode` ABI test; `ep9_facts` tests and schema history; `ep9_vm` IC and feedback tests; SP-Facts in Spike C (M1) |
| Plan sources | [02 §6 H16](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [02 §3](../plan/02-architecture.md#3-fact-sources), [04 §6](../plan/04-execution.md#6-shared-optimizer-the-ir-fact-model-h30), [05 §9](../plan/05-aot.md#9-what-aot-compiles-d19), [09 §2.2](../plan/09-milestones.md#22-scope-and-exit-criteria) |

## Context

- T0 writes feedback; T1, T2 and both AOT kinds read it as a FactSnapshot. Baseline-AOT ICs write the same schema (H33). PGO profiles (`.ep9prof`) and TS hints (HINTED) use it too, so the three fact sources are interchangeable inputs (H30).
- Depends on: H2 (slot kinds are ABI bytes), H8 (ShapeIds are run-local; IC keys hold weak shape pointers), H12 (no address identity), H13 (stable indices), H17 (shapes), H18 (elements kinds).
- Depended on by: H29 (profiling), H30 (fact lattice), H33 (AOT feedback rule), H35 (unit header `fact_schema`), H37 (HINTED entries).
- Goals: G2-G4 (one-compare IC guards), G11 (dual-mode contracts from M0).

## Decision

Copied from the plan row: FeedbackVector after 8 calls. v1 slot kinds. CacheIR-like guard/action lists; mono → ≤ 4-way poly → megamorphic stub cache. IC state only in RW data; no code patching in JIT or AOT; optional embedded stub corpus. PGO profiles, hints and Warp-style snapshots share the schema; baseline-AOT ICs write it.

The week-4 decisions below fill the parts the plan defers (kind **fill**; decisions-log §11).

| # | Decision | Why |
|---|---|---|
| K1 | **Slot kinds v0** = the 21 existing `FeedbackSlotKind` bytes 0-20 and names. No ABI bump. `RuntimeId` stays a placeholder (19 ids) | Each slot operand maps to one kind. The ABI fingerprint hashes names and bytes, not docs; a rename re-blesses 88 goldens and the corpus digest for no gain |
| K2 | **Runtime record**: one 16-byte entry `[key \| value]` per slot. The key is weak, the value strong. A new entry is `[undefined \| int32 0]` | One entry format for T0, JIT and AOT RW cells (OM-5, H8) |
| K3 | **IC states** (OM-6): key undefined = uninitialized; a shape cell = monomorphic; the hole with a POLY_IC cell (≤ 4 cases) = polymorphic; the hole with an int32 = megamorphic, sticky | V8's polymorphism limit is 4 |
| K4 | **Handlers v0** (OM-7): int32 handlers for own data load and store; IC_HANDLER cells for prototype load, missing property and add-property; array and string length by kind; accessor and dictionary receivers record the state and take the slow path | Covers richards and deltablue (M0 exit 4) |
| K5 | **Invalidation v0** (OM-8): one per-isolate `proto_epoch`, bumped on any structural change of a prototype; prototype-chain handlers store and compare it. M1 replaces it with validity cells | Correct, about 3 instructions on a hit |
| K6 | **Vector** (OM-12, OM-13): FEEDBACK cell (kind 68) per function, owned by a per-unit strong root range; made on the 8th call or the first JumpLoop budget poll; `--feedback-threshold` 0 = eager, u32::MAX = never; fp-3 keeps new.target | V8 allocates after 8 calls; eager mode lets test262 run the ICs |
| K7 | **GC** (OM-14, plan rule 8): in IC and feedback data every heap reference is weak, except the value half of a feedback entry and an IC_HANDLER's own cell | IC data must not keep shapes or prototypes alive |
| K8 | **FactSnapshot v0** = `ep9_facts`, `FACT_SCHEMA_VERSION` 1 (below) | Position-independent facts for `.ep9prof` and AOT (H29, H30, H33) |

Per kind (byte, opcodes, runtime record, fact record). "IC" is the K3 entry; "bits" is an int32 bitset in the value word.

| Byte | Kind | Opcodes | Runtime record | Fact record |
|---|---|---|---|---|
| 0 | Call | 0x5e-0x67, 0x69-0x6b, 0x8f call | callee (weak) or hole / int32 count | CallFacts: count, IC state, ≤ 4 FunctionRef, result TypeSet |
| 1 | LoadNamed | 0x21, 0x22, 0x8f load | IC | PropertyFacts: state, ≤ 4 (ShapeDesc, Access), value TypeSet |
| 2, 4, 6 | LoadKeyed, StoreKeyed, DefineKeyedOwn | 0x23; 0x26; 0x27, 0x29 | IC | KeyedFacts: PropertyFacts + KeySet |
| 3, 5 | StoreNamed, DefineNamedOwn | 0x24; 0x25 | IC | PropertyFacts |
| 7, 8, 9 | LoadGlobal, LoadGlobalInsideTypeof, StoreGlobal | 0x10; 0x11; 0x12 | holder shape (weak) / handler | GlobalFacts: BindingSet, value TypeSet |
| 10 | HasKeyed | 0x53 | IC | KeyedFacts (OwnField, ProtoField or Missing) |
| 11 | BinaryOp | 0x2a-0x45, 0x59, 0x5a | - / BinaryHint bits | BinaryHint |
| 12 | CompareOp | 0x4b-0x50 | - / CompareHint bits | CompareHint |
| 13 | TypeOf | 0x48 | - / TypeSet bits | TypeSet |
| 14 | InstanceOf | 0x52 | constructor (weak) / int32 state | TargetFacts: state, ≤ 4 FunctionRef |
| 15 | Literal | 0x6c, 0x6d, 0x6e, 0x70, 0x73 | last shape (weak) / int32 count | LiteralFacts: count, ShapeDesc, ElementsKind |
| 16 | CloneObject | 0x72 | source shape (weak) / handler | CloneFacts: state, ≤ 4 (source, result) ShapeDesc |
| 17 | StoreInArrayLiteral | 0x28 | IC | KeyedFacts (Element cases) |
| 18 | ForIn | 0x8c, 0x8d | - / ForInHint bits | ForInHint |
| 19 | JumpLoop | 0x7d | - / int32 back-edge count | LoopFacts |
| 20 | CreateClosure | 0x74 | the one closure (weak) / int32 0, 1, 2 | ClosureFacts: none, one, many |

T0 v0 writes only the IC entries of 0x21, 0x22, 0x24 and 0x25 (P3-IC); the other records are written from M1 (H16-Q2).

**FactSnapshot v0** (full text: `crates/ep9_facts/tests/golden/schema.txt`):

```text
header 64 B: magic "\x89EP9FS\r\n" | schema_version | bytecode_abi | unit_hash[32] | source | 0 0 0
             | shape_count | record_count | total_size              (u32 little endian)
shapes:  distinct ShapeDesc {ProtoRef, ObjectKind, flags, capacity u16, [(KeyRef, attrs)]}, ascending
records: (function u32, slot u32, kind u8, payload(kind)), ascending; key = (unit_hash, function, slot)
ProtoRef  = null | intrinsic index (H13) | prototype of FunctionRef | unknown
FunctionRef = js (unit hash, function id >= 1) | builtin (BuiltinId u16)
KeyRef    = string (Latin-1 or UTF-16) | well-known symbol | other symbol
```

- No type can hold a ShapeId, an address, a cell kind id or a closure identity. The VM exporter (month 2) turns a live shape into a ShapeDesc.
- Bitsets (TypeSet, BinaryHint, CompareHint, ForInHint, KeySet, BindingSet) only grow and merge by OR. H30 sets precedence between sources.

## Rules

| Rule | Check | From |
|---|---|---|
| Slot kinds are ABI bytes 0-20; a change bumps `ABI_VERSION` and `FACT_SCHEMA_VERSION` | `ep9_bytecode/tests/abi.rs`; `ep9_facts/tests/schema.rs` | M0 wk4 |
| A new slot kind does not compile until it has a payload | exhaustive matches: `Payload::of`, `SlotFacts::empty`, the decoder | M0 wk4 |
| Any format change bumps `FACT_SCHEMA_VERSION` | `schema-history.txt`: SHA-256 of `schema.txt` + the sample bytes; refuses a changed fingerprint even under `EP9_BLESS` | M0 wk4 |
| `ep9_facts` depends on `ep9_bytecode` only; no unsafe | dep-guards check 7; `#![forbid(unsafe_code)]` | M0 wk4 |
| One encoding per fact set; the same facts in any build order give the same bytes | `tests/encoding.rs` (two build orders, corrupt input, 3,000 random round trips) | M0 wk4 |
| A record whose kind is not the unit's `feedback_slots[i]` is dropped; another ABI or unit hash drops the snapshot | `tests/validate.rs` | M0 wk4 |
| IC and feedback data hold shapes weakly | `ep9_vm` tests: a dead shape clears its entry (P2-FEED, P3-IC) | M0 wk4 |
| ICs never change test262 results | T262 and T262-EAGER give the same sets | M0 wk4 |
| No code patching: IC state lives in RW data | review of JIT and AOT IC code | M5 / A3 |

## Consequences

- **Gains.** One IC entry and one fact schema for T0, JIT, AOT and profiles. No ABI bump in week 4: the 88 BytecodeGen goldens and `abi-history.txt` stay.
- **Costs.** A ShapeDesc is larger than a ShapeId, and it is a prediction: two live shapes can share one description (for example with an `unknown` prototype), so a consumer always guards. Per-function feedback merges closures (H16-Q1).
- **Follow-ups.** The VM exporter and `.ep9prof` (H16-Q5); the H13 index lists that `intrinsic` and `builtin` refs name (H16-Q4); H30 merge rules.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R06] | A W^X toggle pair costs about 23 ns, so data-driven ICs beat code patching | [06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R07] | Kiesel got 20-30% from ICs; shapes plus ICs as shared data for later tiers | [07-rust-engine-landscape.md](../research/07-rust-engine-landscape.md) |
| [R14] | Static Hermes AOT keeps `{clazz, slot}` RW IC entries; weval's CacheIR stub corpus reaches 2.77x its interpreter | [14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R17] | ShapeIds are stable only for snapshot root shapes; other guards use RW cells | [17-aot-refutation.md](../research/17-aot-refutation.md) |
| V8 | `FeedbackSlotKind`, allocation after 8 calls, polymorphism 4, BinaryOperationHint, CompareOperationHint, ForInHint | github.com/v8/v8 `src/objects/feedback-vector.h`, `flag-definitions.h` |

## Validation

- `cargo test -p ep9_facts`: round trip, golden bytes, history, wrong-kind drop, two build orders. Negative controls on 2026-10-21: a 22nd kind fails to compile in 3 matches; a renamed kind and an encoder change fail the history test under `EP9_BLESS=1`.
- SP-Facts in Spike C (M1): one kernel compiled from a PROFILED FactSnapshot gives identical output at ≥ 90% of live-feedback speed.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H16-Q1 | Per-closure FeedbackCell (V8) or per-function feedback | M1 | Per-function; revisit if realms or contexts make sites polymorphic |
| H16-Q2 | When T0 records BinaryOp, CompareOp, Call and the other non-property kinds | M1 | M1, with 09 §2.2 "T0 records every H16 slot kind" |
| H16-Q3 | Megamorphic stub cache layout | M1 | A per-isolate (ShapeId, key) table flushed at every GC (H8) |
| H16-Q4 | The index lists behind `intrinsic` and `builtin` refs | M1 (H13 lists) | H13 stable indices; BuiltinId = block (7 bits) + index (9 bits) |
| H16-Q5 | VM exporter (FeedbackVector → FactSnapshot) and the `.ep9prof` container | Month 2; A3 | Walk a live shape to its root to build the ShapeDesc |

## Reopens if

- SP-Facts misses its gate (09 §4).
- M1 needs a slot kind before the freeze: one bump, by the steward (bytecode ABI and fact schema together).
- Per-function feedback turns sites polymorphic much more often than V8 (H16-Q1).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | H16 v0: the 21 slot kinds kept with no ABI bump, a runtime and a fact record per kind; FactSnapshot v0 in `ep9_facts` (FACT_SCHEMA_VERSION 1, schema history); K2-K7 copied from the week-4 plan (OM-5 to OM-8, OM-12 to OM-14) | P2-FACTS (AOT senior role) |
