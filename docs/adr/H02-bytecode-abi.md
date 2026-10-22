# H02: Bytecode is a serialized ABI

| Field | Value |
|---|---|
| Status | Draft (2026-10-21): bytecode ABI v1 and CompiledUnit v0 built in `ep9_bytecode` |
| Fix by | M0 wk3; determinism CI M0; Spike C checks before M1 exit |
| Freeze point | M1 exit |
| Origin | v1#9 + R16-D1; plan status E |
| Crates | `ep9_bytecode` (format, builder, verifier, disassembler); users: `ep9_frontend` (BytecodeGen), `ep9_vm` (interpreter), `ep9_unit` (bytecode section) |
| Owner (plan role) | AOT senior (ABI, determinism); VM lead (interpreter use) |
| Validated by | `ep9_bytecode` tests (gates below); determinism CI on the corpus (M0 exit 12); Spike C |
| Plan sources | [02 §6 H2](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [01 §5](../plan/01-front-end.md#5-front-end-boundary-and-compiledunit-h1-h2), [04 §1-2, §9](../plan/04-execution.md), [05 §8](../plan/05-aot.md#8-aot-unit-format-h35), [H35](H35-aot-unit-format.md) |

## Context

- Every tier and code kind deopts to bytecode, and units ship it (H35 bytecode section). So the bytecode is an ABI between BytecodeGen, T0, T1, T2, the deoptimizer and both AOT kinds.
- The same bytes serve the code cache and AOT units (H1). Determinism lets CI compare builds by hash.
- SP-Dispatch (H15) proved the DSL style: one opcode list, expanded per dispatch method. This ADR fixes the list itself.

## Decision

Copied from the plan row: register + accumulator, 1-byte opcodes, wide prefixes, feedback-slot operands, relative jumps, side tables, constant-pool indices only, `TailCall` reserved. Deterministic generation (same decoded source + flags + ABI version → same bytes), content hash, versioned ABI. Bytecode offset = the only deopt/OSR id. Stable function id = preorder index.

The format decisions below fill the gaps the plan leaves (kind **fill**).

| # | Decision | Why |
|---|---|---|
| F1 | One list, `for_each_opcode!`: name, byte, snake name, accumulator use, flags (`Prefix Jump Switch Terminator Return Throws Call Interrupt Clobber Reserved Generator`), operands with kind and parameter. 158 opcodes (0x00-0x9d, ABI 3). Encoder, decoder, verifier, disassembler, builder emitters and the VM DSL all expand it | One source; a test drives an external consumer from the macro alone |
| F2 | Names and operand order follow V8 Ignition `bytecodes.h` (v8/v8 0c3940db, read 2026-10-20) where ep9 shares the opcode | Proven register + accumulator design ([R06] HTC2) |
| F3 | V8 operand scales: scalable operands are 1 / 2 / 4 bytes after no prefix / `Wide` / `ExtraWide`; fixed operands (`Flag8`, `Flag16`, `Runtime`, `TypeOfLiteral`) keep their size. Little endian. Signed kinds sign-extend | Most operands fit 1 byte |
| F4 | Canonical encoding: an instruction uses the smallest scale that holds all its operands. The builder relaxes jump sizes to a fixpoint; the verifier rejects a larger prefix | One encoding per program, so equal programs hash equal |
| F5 | Register operand = signed name: `v >= 0` local `r<v>`; `-1` receiver; `-2-k` parameter `a<k>`. Register lists, pairs and triples hold locals only. The VM maps names to frame slots | Bytecode does not depend on the H14 frame layout, which Spike B may still change |
| F6 | Jumps: unsigned offset from the instruction start (prefix included). Only `JumpLoop` goes backward (or to itself); every other jump and every jump-table entry goes forward. No V8 `*Constant` jump forms | Every loop has an interrupt poll = an H29 lazy-deopt point. Relaxation replaces constant-pool jumps |
| F7 | Feedback-slot operands on arithmetic, compares, property access, calls, literals, closures, loops. Each function lists its slot kinds; the verifier checks each operand's required kind. `FeedbackSlotKind` is the H16 v0 list (21 kinds, bytes 0-20; [H16](H16-ics-and-feedback.md)) | H16 freezes the kinds at M1 exit; V8's embedded arithmetic feedback is not copied |
| F8 | Constant pool per function (V8 model). Strings live in one unit string table (Latin-1 or UTF-16, canonical form, lone surrogates allowed); pool entries refer to them. Kinds: Number (canonical NaN), String, BigInt (canonical limbs), Function (direct child only), Scope, JumpTable, Array/ObjectBoilerplate (refer only to earlier entries), TemplateObject, Oddball | Strings are atomized once per unit; no cycles; one encoding per value |
| F9 | Side tables per function: handler table (start, end, target, context register, catch prediction; sorted start asc / end desc; nested or disjoint; innermost = last match) and position table (bytecode offset → UTF-8 source offset + statement bit, delta + minimal LEB128). Line and UTF-16 column are computed lazily by `SourceLines` | 01 §5: UTF-8 offsets stored, UTF-16 map built lazily |
| F10 | Reserved: `Illegal` = 0x00 (zeroed memory never runs) and `TailCall` (D7). The verifier rejects them. The generator opcodes `SwitchOnGeneratorState` / `SuspendGenerator` / `ResumeGenerator` carry the `Generator` flag (ABI 3): the verifier allows them only in a function with the `GENERATOR` or `ASYNC` flag (`VerifyErrorKind::GeneratorOpcode`) | Layout is fixed; enabling the generator opcodes was the ABI 3 bump |
| F11 | `LdaCurrentContext` (new, not in V8): ep9 has no `<context>` register name, so BytecodeGen saves the context for a handler with `LdaCurrentContext; Star r` | Found by the sample unit; F5 keeps special frame slots out of register names |
| F12 | Content hash = SHA-256 of the unit with the hash and build-id fields zeroed. Source hash = SHA-256 of the decoded UTF-8 text. Header byte names the algorithm (1) | Collision resistant (a D6 signature can sign it), checkable with `shasum -a 256`, no dependency |

## CompiledUnit v0 layout

All integers little endian. Header 168 bytes (ABI 5; 160 in ABI 1-4); then 6 sections back to back (5 in ABI 1-4). Bytes 0..20 are a frozen prefix for every ABI version.

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | magic `89 45 50 39 42 43 0D 0A` ("\x89EP9BC\r\n", PNG-style) |
| 8 / 12 / 16 | 4 each | ABI version (5) / header size (168) / total size |
| 20 | 4 | flags: bits 0-1 code kind (script, module, eval); 2 strict; 8-15 TS ([H03](H03-ts-runtime-semantics.md)); 16 has source; 17 has hints |
| 24 / 25 | 1 / 7 | hash algorithm (1 = SHA-256) / reserved, zero |
| 32 / 64 / 96 | 32 / 32 / 16 | content hash / source hash / engine build id (not hashed) |
| 112 / 116 | 4 / 4 | source length in bytes / reserved, zero |
| 120 | 48 | section table: (offset, length) for strings, scopes, functions, source, hints, module records (index 5, ABI 5) |

Check order: magic, ABI version, header layout, content hash, then structure, then the verifier. The ABI version comes before the hash, so a unit of another ABI ends in SOURCE even if a later ABI changes the header or the hash rule. For a code-cache entry, SOURCE and LOAD ERROR both recompile the source file.

Function record: kind, flags, name, parent, outer scope, source start/end, parameter count, `length`, register count, then bytecode, constants, feedback kinds, handlers, positions (each with a u32 count), then (ABI 5) the mapped-parameter list: scope id + 1 (0 = no list) and a u32 count of slot entries, each a context slot + 1 (0 = a parameter that a later duplicate name shadows). Optional ids store `id + 1` (0 = none). `tests/golden/tiny-unit.hex` pins the bytes.

**Module records (section 5, ABI 5).** Empty for a script or eval unit; never empty for a module unit (the verifier checks both). It holds the records of ECMA-262 §16.2.1.6 ParseModule, every list in source order: a flag byte (bit 0 = `[[HasTLA]]`); the requested modules (specifier string id, then (key, value) string id pairs of the import attributes, sorted by key in UTF-16 code unit order, keys unique; one entry per distinct pair of specifier and attributes); the import entries (request index, import name string id + 1 with 0 = namespace-object, local name, module cell i32); the local export entries (export name, local name, module cell); the indirect export entries (export name, request index, import name string id + 1 with 0 = `all`); the star export entries (request index). The export entries are sorted as ParseModule step 10 does: a re-export of an imported binding is indirect, a re-export of an imported namespace object stays local. Module cells are the BytecodeGen v0 numbering (exports `1, 2, ...`, imports `-1, -2, ...`); the verifier checks only that a cell is not 0. The import phases of the stage 3 proposals (`import source`, `import defer`) are not recorded. `tests/golden/sample-module.txt` shows every kind.

## Rules

| Rule | Check | From |
|---|---|---|
| The opcode table is the golden `opcodes.txt` | `tests/golden.rs::opcode_table` | M0 |
| A table or layout change bumps `ABI_VERSION`: `abi-history.txt` maps each version to one fingerprint (SHA-256 of `opcodes.txt` + tiny unit + runtime table) | `tests/abi.rs` | M0 |
| A slot-kind change also bumps `FACT_SCHEMA_VERSION` (H16): the fact schema lists every kind | `ep9_facts/tests/schema.rs` (`schema-history.txt`) | M0 wk4 |
| Same builder input → same bytes and hash, on every thread and target; function order does not matter | `tests/determinism.rs`; `hashes.txt` checked on macOS arm64/x64 and Linux arm64/x64 | M0 |
| serialize → deserialize → verify round-trips; any input that loads re-serializes to the same bytes; corrupt input fails without a panic | `tests/roundtrip.rs` (random units, byte flips, rehashed mutations, truncation) | M0 |
| The verifier enforces the guarantee table in `verify.rs` | `tests/verifier.rs`, one test per rule | M0 |
| Disassembly text is stable | `sample-script.txt`, `sample-module.txt` | M0 |
| ABI 5 parts: `PARAMETER_ENV` only with `CALLS_SLOPPY_EVAL` on a sloppy `Function` scope; a mapped-parameter list only in a function with `CreateMappedArguments` that makes a context of the list's `Function` scope, one entry per formal parameter, each slot a `var` of that scope, no slot twice; module records present exactly in a module unit, indices and string ids in range, attribute keys sorted and unique | `tests/verifier.rs` `abi5_parameter_env_rules`, `abi5_mapped_parameter_rules`, `abi5_module_record_rules`, `abi5_runtime_arity` | M3-P0 |
| Load errors map to H35 outcomes: bad magic, header or hash → LOAD ERROR; another ABI version → SOURCE | `roundtrip.rs::header_errors_map_to_h35_outcomes` | M0 |

**Entry-frame rule (SP-Dispatch lesson).** The spike's entry function had no caller header, so `Return` from it read garbage. In ep9 the host entry writes a full H14 header, so `Return` from function 0 returns to the host. The verifier enforces the bytecode side: only function 0 has an entry kind (= the unit's code kind); it has 0 parameters and no `CreateMappedArguments`, `CreateUnmappedArguments`, `CreateRestParameter` or `LdaNewTarget`; it is never a closure target.

## Consequences

- **Gains.** BytecodeGen gets a typed builder; the VM gets a verified unit it can decode without bounds checks. Units and the code cache share one canonical format.
- **Costs.** Load speed on Apple M5 Max, 6.7 MB of random units, release: hash 440 MB/s, hash + decode 295 MB/s, + verify 184 MB/s. Portable SHA-256 is 422 MB/s; hardware SHA-256 (OpenSSL) is 3,316 MB/s. The hash is about 40% of load time.
- **Follow-ups.** BytecodeGen (01 §5); interpreter DSL on `for_each_opcode!` (H15); determinism CI on the corpus; ADR index update.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R06] | Register machine + accumulator: about 46% fewer dispatches than a stack VM | [06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R14] | Bytecode is a serialized ABI and the universal deopt target; it ships in every unit, inlined callees included (Graal Ristretto lesson) | [14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| V8 | `src/interpreter/bytecodes.h`, `bytecode-operands.h` at 0c3940db | github.com/v8/v8 |
| SP-Dispatch | Entry frame must not `Return` without a header; verifier lesson | [spikes/sp-dispatch](../../spikes/sp-dispatch/README.md) |

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H02-Q1 | The plan row lists serialized liveness. v0 has no liveness section | Spike C (before M1 exit) | Add a per-function section, computed from `AccUse` and register operand kinds |
| H02-Q2 | Context slot/depth operands are checked only against a unit-wide bound | M1 (closures in T0) | Abstract-interpret the context chain in the verifier, or bounds-check in the handler |
| H02-Q3 | Module records (requests, imports, exports) are not in v0 | `compile_module` (M1) | Filled 2026-10-22 (ABI 5, M3-P0): section 5, "Module records" above. The loader reads it from M3-P2 (MODULE) |
| H02-Q4 | Load speed for A1 (typescript.js ≥ 2x faster from a unit) | A1 (M4) | Hardware SHA-256 path in a crate that allows unsafe, or lazy per-function verification |
| H02-Q5 | `RuntimeId` is a placeholder (48 ids, 0-47; ABI 3 added 19-28, ABI 4 added 29-31, ABI 5 added 32-47). The `FeedbackSlotKind` half was filled on 2026-10-21: H16 v0, no bump | H13 index lists (M1) | Replace with the H13 runtime table; bump ABI |
| H02-Q6 | Short `Star0-15` forms and superinstructions (V8 has short stars) | M2 dispatch tuning | Add only with a dispatch-count measurement |
| H02-Q7 | Lazy compile (H4) needs a per-function result type | L1 (M4) | Reuse `Function` + a scope chain record |

## Reopens if

- Spike C cannot deopt to bytecode offsets or needs data the format cannot carry.
- SP-Unit (H35) needs a different bytecode section shape.
- H14 (Spike B) needs register names that F5 cannot map in one step.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | ABI v1 opcode list, CompiledUnit v0 layout, verifier, disassembler, determinism and round-trip tests | bytecode agent |
| 2026-10-21 | ABI bump | ABI 2 for BytecodeGen v0: runtime ids 6-18 (`InitializeGlobal` ... `StoreToSuper`); `DefineClass` takes 2 arguments (constructor, heritage). The ABI fingerprint now includes `runtime_table_text()`, so a runtime table change also bumps the ABI. `abi-history.txt` row 2 | bytecodegen agent |
| 2026-10-21 | API addition | `FunctionBuilder::set_throw_position(Option<u32>)`: a default position for throwing instructions that have no pending position. No format change | bytecodegen agent |
| 2026-10-21 | evidence | H02-Q2: a test-only context-chain check (`ep9_frontend/src/bytecodegen/ctx_check.rs`) follows context values through each function. 0 errors on 72,147 BytecodeGen units (test262 + bench corpus). It is a candidate for the verifier | bytecodegen agent |
| 2026-10-21 | rewrite | PR #1 review fix B1: a handler target must be at or after its range end. A throw then only goes forward (as F6 says for jumps), and every loop inside a function passes a `JumpLoop` interrupt poll. Before, a handler back edge could loop through `Throw` with no poll, and termination did not stop it. Chosen over a poll on the unwind path: no new poll or deopt point, no VM change. The verifier now rejects what it accepted, but the format does not change and ABI 1 and 2 never shipped, so `ABI_VERSION` stays 2. 72,147 corpus units still verify ([decisions-log §10](../decisions-log.md#10-2026-10-21-pr-1-review-fixes-ci-gates-and-h15-on-t0) B1) | PR #1 review fix (Codex review) |
| 2026-10-21 | rewrite | PR #1 review fix B2: the verifier checks the reader's rules (canonical strings; defined scope and function flag bits) through `UnitString::is_canonical` and `ScopeFlags`/`FunctionFlags::is_defined`, which the reader uses too. So a unit from `UnitBuilder::finish` always loads back (decisions-log §10 B2) | PR #1 review fix (Codex review) |
| 2026-10-21 | fill | H02-Q5 split. `FeedbackSlotKind` becomes the H16 v0 list: bytes 0-20 and names unchanged, only doc text, so no ABI bump (`abi-history.txt` stays `2 6d92eb9a`; goldens unchanged). `RuntimeId` stays a placeholder with 19 ids (0-18) until the H13 lists (M1). F7 names H16 v0. New rule: a slot-kind change also bumps `FACT_SCHEMA_VERSION` | P2-FACTS (ABI steward) |
| 2026-10-21 | clarify | The fingerprint rule names the runtime table too (part of the fingerprint since ABI 2, as the ABI 2 row says) | P2-FACTS (ABI steward) |
| 2026-10-21 | rewrite (ABI bump) | ABI 3 (M2 P0). **F10:** `SwitchOnGeneratorState`, `SuspendGenerator` and `ResumeGenerator` lose `Reserved` and get a new opcode flag `Generator` (bit 10); the verifier accepts them only in a function with the `GENERATOR` or `ASYNC` flag (`VerifyErrorKind::GeneratorOpcode`), in M0 instead of M1. Typed builder emitters exist for them now. **F1:** 158 opcodes (0x00-0x9d). `CallRuntimeForPair` = 0x9d, `(id: Runtime, args: RegList, argc: RegCount, out: RegOutPair)`, acc None, as in V8. New operand kind `RegOutPair` = 20: two consecutive locals the instruction writes (signed, scalable, a register); the verifier checks that both are locals inside the frame, and `ctx_check` marks both as written. **Flags:** the `DefineKeyedOwnProperty` mask goes 0x01 -> 0x03 (bit 1 = private add: throw if the key is present). `DefineKeyedOwnPropertyInLiteral` keeps 0x01 (no private names in a literal). **H02-Q5:** runtime ids 19-28 with fixed arities: `CreateGeneratorObject` 2, `GeneratorGetResumeMode` 1, `GeneratorClose` 1, `ThrowThrowMethodMissing` 0, `CreatePrivateName` 1, `SetClassFieldsInitializer` 2, `RunClassFieldsInitializer` 2, `LoadLookupSlotForCall` 1, `DeclareEvalVar` 1, `DeclareEvalFunction` 2. `RuntimeInfo::returns_pair` marks `LoadLookupSlotForCall`; the runtime table text adds ` pair`, and the verifier allows a pair function only through `CallRuntimeForPair` and every other one only through `CallRuntime` (`VerifyErrorKind::RuntimePair`). **Convention (not a verifier rule):** slot 0 of each `CALLS_SLOPPY_EVAL` function var scope is the `.eval_ext` binding (`EVAL_EXT_BINDING`), never in a `Script` scope. `abi-history.txt` row `3 4ce2320d…`; the 88 BytecodeGen goldens and 3 `ep9_bytecode` goldens re-blessed (the ABI header line; `call-eval` and `var-direct-eval` also get the `.eval_ext` slot). The new corpus digest is the orchestrator's BCG-CORPUS-4 row | P0-LANG (ABI steward) |
| 2026-10-21 | clarify | Body rows F1 (158 opcodes, 0x00-0x9d, flag `Generator`), F10 (generator opcodes allowed in generator and async functions) and H02-Q5 (29 runtime ids, 0-28) now state the ABI 3 values of the row above | M2-P0 integration fixer |
| 2026-10-22 | rewrite (ABI bump) | ABI 4 (M2-P2 review fix L1, plan §10 procedure). **H02-Q5:** runtime ids 29-31 with fixed arities, none a pair: `ResolveLookupSlot` 1 (ResolveBinding of a dynamic name without GetValue; returns the environment: a context, a `with` object, undefined for the global environment, null for an unresolvable name), `LoadLookupSlotResolved` 2 (`env, name`: GetValue of that reference) and `StoreLookupSlotResolved` 4 (`env, name, value, strict`: PutValue to it; returns the value). BytecodeGen resolves a dynamic target before the right-hand side of `=`, `op=`, a logical assignment, `++`/`--` and a `var` initializer (§13.15.2, §13.4, §14.3.2.1). No opcode, operand or layout change. `abi-history.txt` row `4 0e42ef8d…`; the BytecodeGen goldens and the 3 `ep9_bytecode` goldens re-blessed (the ABI header line only). The M3-P0 bump (plan §10 "ABI 4") becomes ABI 5, as M2-ABI-FIX foresees. The new corpus digest is the orchestrator's | M2-P2 review fixer (lookup) |
| 2026-10-22 | rewrite (ABI bump) | ABI 4, second part (M2-P2 review fix L4, R2): scope slot mode `VariableMode::FunctionName` = 5 (`function-name` in the disassembly), the own name of a named function expression, an immutable binding that is not strict. The reader accepts mode 5 from ABI 4. BytecodeGen gives it to `BindingKind::FunctionName` (was `Var`), and the VM's dynamic store ignores a sloppy store to it and throws a TypeError for a strict one. The ABI fingerprint does not cover slot modes, so `abi-history.txt` keeps `4 0e42ef8d…`; the random-unit hash in `hashes.txt` is re-blessed (the generator now picks mode 5) | M2-P2 review fixer (lookup) |
| 2026-10-22 | rewrite (ABI bump) | ABI 4, third part (M2-P2 round-2 review fix L6; ABI 4 is not yet published, so no ABI 5). GetValue of a dynamic name takes the strictness of the use site (§9.4.2 step 3), and a class heritage or computed key is strict code inside sloppy code (§11.2.2). **H02-Q5:** `LoadLookupSlotForCall` (id 26, a pair) takes 2 arguments (`name, strict`), `LoadLookupSlotResolved` (id 30) takes 3 (`env, name, strict`); `strict` is true or false, as for id 31. No opcode change: BytecodeGen emits `LdaLookupSlot` / `LdaLookupSlotInsideTypeof` (which read with the function's strictness) only where the two agree, and a strict read in a sloppy function takes ids 29 + 30. `abi-history.txt` row 4 becomes `4 6ee2245d…` (was `4 0e42ef8d…`, which the two rows above name); `hashes.txt` random-unit hash and the `var-with-call` BytecodeGen golden re-blessed. The new corpus digest is the orchestrator's | M2-P2 round-2 review fixer (lang) |
| 2026-10-22 | rewrite (ABI bump) | ABI 5 (M3-P0, plan-v7 §10 procedure; M3-PLAN-D11, D20, D36). **H02-Q5:** runtime ids 32-47 with fixed arities, all single-result: `AsyncFunctionEnter` 2, `AsyncFunctionAwait` 2, `AsyncFunctionResolve` 2, `AsyncFunctionReject` 2, `AsyncGeneratorEnter` 2, `AsyncGeneratorAwait` 2, `AsyncGeneratorYield` 2, `AsyncGeneratorResolve` 3, `AsyncGeneratorReject` 2, `CreateAsyncFromSyncIterator` 2, `GetImportMeta` 0, `DynamicImport` 2, `SetFunctionName` 2, `ThrowTypeError` 1, `DeclareEvalAnnexBFunction` 1, `StoreEvalAnnexBFunction` 2 (B.3.2.3: SetMutableBinding on the eval code's variable environment). In M3-P0 BytecodeGen emits only 44 (a class under a computed key; it replaces the scratch-object define, M2-P2-C2-DEFER) and 45 (a store to a private method, text "Private method is not writable"; it replaces `ThrowConstAssignError`, M2-P2-OPEN-ROLES), and the VM routes only those two. **Scopes:** `ScopeFlags::PARAMETER_ENV` = 1 << 3 (`ALL` 0x07 -> 0x0F; H04), the verifier accepts it only with `CALLS_SLOPPY_EVAL` on a sloppy `Function` scope. **H02-Q3 filled:** the module records section, index 5 (header 160 -> 168 bytes, 6 sections, frozen prefix unchanged). **Mapped-parameter list** at the end of each function record (P0-D16): the scope and, per formal parameter, its context slot or none; one more scope id than the plan text names, so the verifier checks each slot against that scope (it must be a `Function` scope that the function makes a context of). No opcode or operand change. `abi-history.txt` row `5 fa6d4dad…`; `hashes.txt`, `sample-module.txt` (header line, module records of every kind), `sample-script.txt` and `tiny-unit.hex` re-blessed (`opcodes.txt` is unchanged: the runtime table is in the fingerprint, not in that file); the 103 BytecodeGen goldens re-blessed (the ABI header line; module goldens get their records; `class-private-assign` shows id 45; mapped functions show their list) plus 4 new ones (`class-computed-name` id 44, `module-records`, `fn-mapped-arguments`, `fn-params-eval`). The new corpus digest is the orchestrator's BCG-CORPUS-10 row | P0-LANG4 (ABI steward) |
