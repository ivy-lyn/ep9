# H03: TS runtime semantics in BytecodeGen; TS flags in module identity

| Field | Value |
|---|---|
| Status | Accepted (2026-10-22, M2-P3-ACCEPT; review record complete). Header fields built; lowering table not started (M4) |
| Fix by | Header fields M0 wk3; ADR M0 exit; built M4 |
| Freeze | Header fields and TS flags: M1 exit. BytecodeGen lowering table: M4 exit ([README §3.4](README.md#34-parts-built-after-m1-exit-p-3)) |
| Origin | R16-D14 + R17; plan status N |
| Crates | `ep9_bytecode` (header fields, this draft); `ep9_frontend` (lowering, M4); `ep9_hints` (`.ep9h` header, H37) |
| Owner (plan role) | AOT senior (header fields); lead/front end (lowering) |
| Validated by | `ep9_bytecode` tests (below); M4 lowering tests |
| Plan sources | [02 §6 H3](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [01 §9](../plan/01-front-end.md#9-running-typescript-changed), [05 §7](../plan/05-aot.md#7-side-table-schema-h37-crate-ep9_hints), [H35 S3](H35-aot-unit-format.md) |

## Context

- The same class source gives other object shapes under `[[Set]]` and `[[Define]]` fields. 4 of 6 measured codebases use `[[Set]]` [R12].
- So the TS flags are part of module identity: the unit, the `.ep9h` header and the code-cache key must carry them.
- D20 (decided 2026-10-20): follow tsconfig; without a tsconfig use `[[Define]]`.

## Decision

Plan row: BytecodeGen lowers enums, namespaces, parameter properties, declare fields and `[[Set]]`/`[[Define]]` fields from the TS AST. oxc_transformer only for legacy decorators (no hints in v1). `useDefineForClassFields`, `experimentalDecorators` and `emitDecoratorMetadata` live per module in the CompiledUnit header, the `.ep9h` header and the cache key. Default `[[Define]]` without tsconfig.

This draft fixes only the **header fields** (M0 wk3). The lowering table stays as in 01 §9 until M4.

| Unit flags bit ([H02](H02-bytecode-abi.md) header offset 20) | Meaning | Default without tsconfig (D20) |
|---|---|---|
| 8 `TYPESCRIPT` | The source is TypeScript | - |
| 9 `TS_USE_DEFINE_FOR_CLASS_FIELDS` | Class fields use `[[Define]]` (set) or `[[Set]]` (clear) | set |
| 10 `TS_EXPERIMENTAL_DECORATORS` | Legacy decorators | clear |
| 11 `TS_EMIT_DECORATOR_METADATA` | Decorator metadata | clear |
| 12-15 | Reserved for later TS flags (the M4 ADR may add some) | zero |

Rust API: `UnitHeader::typescript: Option<TsFlags>`; `TsFlags::DEFAULT` is the D20 default.

## Rules

| Rule | Check | From |
|---|---|---|
| A JavaScript unit (bit 8 clear) has bits 9-15 clear | reader: bits 9-11 give `BadHeader("TS flags on a JavaScript unit")`, bits 12-15 give `BadHeader("unit flags")` (row below; the defined-bits check runs first, `crates/ep9_bytecode/src/serialize.rs:821-822`); `roundtrip.rs::header_errors_map_to_h35_outcomes` | M0 |
| Bits 12-15 are zero until an H03-Q2 flag is added (every ABI so far: 1-4) | reader rejects undefined flag bits (`BadHeader("unit flags")`); `crates/ep9_bytecode/tests/roundtrip.rs` `header_errors_map_to_h35_outcomes` sets each bit 12-15 on a JavaScript and on a TypeScript unit | M0 |
| TS flags are covered by the content hash, so another flag value is another module identity and cache key | `determinism.rs::content_hash_excludes_build_id_and_covers_ts_flags` | M0 |
| The flags round-trip | `sample-module.txt` golden (bit 8 and bits 10-11 set, bit 9 clear: the `[[Set]]` case); random units set them at random | M0 |
| H35 S3: a unit's TS flags are not compared at load; SOURCE compiles the source section with them | `ep9_unit` loader (A2) | A2 |
| The `.ep9h` header carries the same three flags; a mismatch drops the hint file | `ep9_hints` (A1h) | A1h |

## Consequences

- **Gains.** The hint join and the code cache cannot mix `[[Set]]` and `[[Define]]` builds of one source. The layout already has room for later TS flags.
- **Costs.** One module source can have up to 8 cached units (one per flag combination).
- **Follow-ups.** M4: lowering table and pass order for decorator files; the ADR index row.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R12] | 4 of 6 codebases use `[[Set]]` fields (target below ES2022 or explicit `false`) | [12-ts-type-soundness.md](../research/12-ts-type-soundness.md) |
| [R17] | oxc_transformer gives lowered fields and enums dummy spans, so hint keys are lost | [17-aot-refutation.md](../research/17-aot-refutation.md) |

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H03-Q1 | Does tsconfig `target` (implied `useDefineForClassFields`) resolve in `ep9_runtime` or in `ep9_frontend`? | M4 | `ep9_runtime` resolves tsconfig and passes final `TsFlags` |
| H03-Q2 | Other TS-only runtime forms found at M4 | M4 | New bits 12-15 + ABI bump |

## Reopens if

- D20 changes (owner override), or M4 finds a TS flag that changes runtime semantics and does not fit bits 12-15.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | Header fields: unit flag bits 8-15, `TsFlags`, reader and hash rules | bytecode agent |
| 2026-10-22 | clarify | Rules: bits 12-15 are zero in ABI 1-3 (was "ABI v1"; ABI_VERSION is 3); the check names its error and says that no test sets those bits yet. The `sample-module.txt` golden sets bit 9 clear, not "all three". No check changes | P2-ACCEPT-1 |
| 2026-10-22 | rewrite | Rules row "Bits 12-15 are zero": its check now names a test. `header_errors_map_to_h35_outcomes` sets each of bits 12-15 on a JavaScript and a TypeScript unit and expects `BadHeader("unit flags")` (review finding R2). The rule and the reader are unchanged; the kind is rewrite because a Rules row and a test change (README §3.3). `serialize.rs` `TS_RESERVED` doc now says ABI 1-3 (R4) | P2 integration fixer |
| 2026-10-22 | clarify | Rules: bits 12-15 are zero in ABI 1-4 (ABI 4 adds runtime ids 29-31 and `VariableMode::FunctionName` = 5, and ids 26 and 30 take a strict argument; no header flag changes; H02 change log); no check changes | M2-P2 review fixer (lookup) |
| 2026-10-22 | clarify | Rules row "Bits 12-15 are zero" now reads "until an H03-Q2 flag is added (every ABI so far: 1-4)" (was "(ABI 1-4)"), so a later ABI bump that adds no TS flag needs no H03 amendment; H03-Q2 adds such a flag with an ABI bump. The 2026-10-22 lookup-fixer row above said "ABI 4 adds runtime ids only"; that was false (ABI 4 also adds `VariableMode::FunctionName` = 5, and ids 26 and 30 take a strict argument: `crates/ep9_bytecode/src/unit.rs:30`, H02 change log), so its text is corrected in place. No check changes | P3-ACCEPT-2 |
| 2026-10-22 | clarify | Rules row "A JavaScript unit has bits 9-15 clear": its check named one error for all 7 bits. The reader gives `BadHeader("TS flags on a JavaScript unit")` for bits 9-11 and `BadHeader("unit flags")` for bits 12-15 (its defined-bits check runs first); `header_errors_map_to_h35_outcomes` checks bit 9 and bits 12-15 that way. No check changes | P3-ACCEPT-2 |
| 2026-10-22 | clarify | Status Draft -> Accepted (M2-P3-ACCEPT). The §3.1 review record is complete: row 1 P2-ACCEPT-1, row 2 Codex (M2-P2-REVIEW), row 3 P3-ACCEPT-2, row 4 data. The Codex run on the P3 ADR slice (merge 6146ec5) had no finding on this ADR. The decision text does not change | orchestrator |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P2-ACCEPT-1 (did not write the ADR) | Decision = the 02 §6 H3 row. Rules checked against `ep9_bytecode` at 383a317 (the `serialize.rs` flag bits and reader, `TsFlags`, the named tests) and H35 S3. R1: "ABI v1" is now ABI 1-3 (fixed). R2: no test sets a bit 12-15 (the check says so; a test case is reported for the fixer). R3: the golden has bit 9 clear, not "all three set" (fixed). R4: the `serialize.rs` doc "zero in ABI v1" is in a frozen ABI file (reported). Change-log kinds: agree | ccaf5df |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | No finding on H03. Codex ran on the "ADR docs" slice (31 KB) of the P2 diff at e293355 (M2-P2-REVIEW). Verdict needs-attention with 2 findings, A1 (H13) and A2 (H27, H36); a verifier reproduced both: real, category (b), fixed in P2 | decisions-log M2-P2-REVIEW, M2-P2-A1, M2-P2-A2; Codex text outside the repo: scratchpad m0m2/p2-codex-bcrhkdqte.md |
| 3 Verifier | P3-ACCEPT-2 (wrote none of the rows it checks) | Checked at e719ffa (tag m2-p2 5c9034a + one gate.sh commit). No Codex finding to sort. Rows written by others: the P2-ACCEPT-1 clarify row (the `sample-module.txt` golden has bits 8, 10 and 11 set and bit 9 clear: true); the fixer rewrite row (c6bf181: `header_errors_map_to_h35_outcomes` sets each of bits 12-15 on a JavaScript and a TypeScript unit and expects `BadHeader("unit flags")`; the reader's defined-bits check is `crates/ep9_bytecode/src/serialize.rs:821-822`; the `TS_RESERVED` doc says ABI 1-4: true; kind rewrite: agree); the lookup-fixer ABI 4 row. V1: that row said "ABI 4 adds runtime ids only", which is false (`VariableMode::FunctionName` = 5 and the strict argument of ids 26 and 30, `crates/ep9_bytecode/src/unit.rs:30`); its text is corrected and Rules row 2 now reads "until an H03-Q2 flag is added (every ABI so far: 1-4)" (M2-P3 plan task 4, clarify). V2: Rules row 1 named one reader error for bits 9-15; bits 12-15 give `BadHeader("unit flags")` (fixed, clarify). Part 4 again at e719ffa: `cargo test -p ep9_bytecode --locked` 75 pass, 2 ignored, 0 fail, with every test the Rules name. Change-log kinds: agree | c68971a; local run, mac a64, e719ffa; logs outside the repo: scratchpad m0m2/accept2/logs/bytecode.log |
| 4 Data | from the "Validated by" line | `cargo test -p ep9_bytecode --locked`: every suite passes, with `header_errors_map_to_h35_outcomes`, `content_hash_excludes_build_id_and_covers_ts_flags`, the golden test and `random_units_round_trip`. Missing: the M4 lowering tests (M4) | local run, mac a64, 383a317; logs outside the repo: scratchpad m0m2/accept1/h03-ep9_bytecode-test.log |
