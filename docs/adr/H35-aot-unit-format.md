# H35: AOT unit format (container spec skeleton)

| Field | Value |
|---|---|
| Status | Draft, spec skeleton (2026-10-20). Header v0, section table and load check (S4 steps 1-4) built in `crates/ep9_unit` (M0 wk4, S6); H35-Q8 filled 2026-10-22 (engine build id); H35-Q2 and H35-Q9 filled 2026-10-23 (M3-P1 SP-UNIT: SHA-256 content hash, signature slot reserved; the unit-data register). SP-Unit is built (`gate.sh unit`; [data](../measurements/2026-10-23-sp-unit.md)); its verdict is the orchestrator's. Writer and loader A2 |
| Fix by | Spec v0 + header struct + golden test by M0 exit; writer/loader A2; frozen at A3 gate |
| Freeze point | A3 gate |
| Origin | R16-D6 + R17; plan status N (new) |
| Crates | `ep9_unit` (container, header, verify, loader; writer under `aot`), `ep9_masm` (unit finalize), `ep9_platform` (mapping, unwind registration) |
| Owner (plan role) | AOT senior |
| Validated by | SP-Unit (M3-P1, moved by M2-D21-FAIL; built on Spike B placement (b)); A2 gate |
| Plan sources | [02 §6 H35](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [05 §8](../plan/05-aot.md#8-aot-unit-format-h35), [04 §5](../plan/04-execution.md#5-jit-infrastructure-and-the-relocatable-macroassembler), [03 §6](../plan/03-gc.md#6-snapshot-interaction) |

## Context

- Units carry AOT code (baseline-AOT, optimized-AOT) and the bytecode it came from. One unit text must serve every isolate in a process.
- iOS needs code inside the signed app (D16). Wasmtime refuses text relocations. Leyden rejects mismatched code [R14][R11].
- A unit is trusted code, like a native addon (D6). A stale or mismatched unit must never crash (RK35).
- This file is the M0 wk1 skeleton. The header struct and golden test come in M0 wk4; spec v0 is due at M0 exit (exit item 10).

## Decision

Zero text relocations. Isolate register + unit-data register for the per-isolate RW data (H35-Q9) + link table. Link and IC cells resolved at load or first execution. ShapeId immediates only for snapshot root shapes. Header: versions, CPU features, content hash (+ optional signature). `.o` or `.ep9u` blob. Mismatch → by field (S3): native-only versions and CPU → the unit's bytecode; engine build id or bytecode ABI → the unit's source section; bad prefix, hash or signature → clean load error.

## Container spec v0 (skeleton)

**S1. Forms.** One container, two forms: an `.ep9u` blob (mmapped, servers) or `.o` sections (linked into a signed app, iOS). The `.o` form uses static registration.

**S2. Sections** (from [05 §8](../plan/05-aot.md#8-aot-unit-format-h35)):

| Section | Access | Sharing | Content | Rows |
|---|---|---|---|---|
| header | R | per process | versions, CPU features, content hash, (signature) | H35 |
| text | RX | per process | baseline-AOT and optimized-AOT code; zero relocations | H34 |
| rodata | R | per process | deopt translations, safepoint + pc->bytecode maps, position tables, function table | H29, H24 |
| RW template | RW | copied per isolate | link table (roots, builtins, atoms, fuses), IC cells, exit + call counters, fuse refs | H13, H16, H31 |
| bytecode | R | per process | every compiled function and inlined callee | H2 |
| source | R | per process | always (Function.prototype.toString, lazy compile) | H5 |
| hints | R | per process | optional HINTED section (`ep9_facts` format) | H30, H37 |
| unwind/debug | R | per process | unwind tables, perf map | H24, H28 |

**S3. Header fields.** Encoding, size and order: S6 (H35-Q1, filled M0 wk4).

| Field | On mismatch (plan) | Row |
|---|---|---|
| Magic, container version, header size, section table bounds | S4 step 1: LOAD ERROR | H35 |
| Content hash, optional signature | S4 step 2: LOAD ERROR | H35, D6 |
| Engine build id | S4 step 3: SOURCE | H35 |
| Bytecode ABI version | S4 step 3: SOURCE | H2 |
| Value-encoding version | S4 step 4: BYTECODE (06 §1) | H6 |
| Object-layout version | S4 step 4: BYTECODE (06 §2.1) | H8 |
| Barrier version | S4 step 4: BYTECODE (03 §3) | H11 |
| Builtin ABI version | S4 step 4: BYTECODE | H20 |
| Heap page size (own field, H35-Q6) | S4 step 4: BYTECODE | H9 |
| Frame, IC, deopt-metadata versions | S4 step 4: BYTECODE | H14, H16, H29 |
| Fact-schema version | S4 step 4: BYTECODE; drop the hints section too | H16, H37 |
| CPU features and target | S4 step 4: BYTECODE | H35 |
| TS flags (`useDefineForClassFields`, `experimentalDecorators`, `emitDecoratorMetadata`) | Not compared for a unit; SOURCE compiles with them. Part of the code-cache key (07 §3) | H3 |

**S4. Load flow** (copied from 05 §8; decision H35-fallback, 2026-10-20):

```
 load unit (.ep9u or .o)                      code cache entry: steps 1-3 only
   +-1 prefix: magic, container version, header size, section table -- fail -----> LOAD ERROR
   +-2 content hash (+ signature if D6 requires) ---------------------- fail -----> LOAD ERROR
   +-3 engine build id, bytecode ABI version ------------------------- mismatch -> SOURCE
   +-4 value-encoding, object-layout, barrier, frame, IC, deopt-metadata,
   |     fact-schema versions; CPU features and target --------------- mismatch -> BYTECODE
   +-5 map text RX, shared per process
   +-6 copy RW template per isolate; resolve link cells (roots, builtins, atoms,
   |     fuses); IC cells start empty; IntrinsicRef reads the current realm
   +-7 register pc->code registry, unwind info, perf map, GC visitor
   +-8 install dispatch slots: aot-baseline / aot-opt
```

| Outcome | Action | Why |
|---|---|---|
| LOAD ERROR | `ep9 run --unit`: clean error, exit code 1, no section used. A statically linked `.o` unit: the same clean startup error, exit code 1. Code cache: drop the entry, compile the source file, write a new entry | A bad prefix hides where the sections are. A failed hash or signature leaves no trusted byte, the source section included. Hermes also rejects a bad magic or bytecode version (`BCProvider.cpp` sanityCheck) |
| SOURCE | Compile the unit's source section (always present, H5) with the header's goal, strict and TS flags -> T0. Drop the bytecode, native code and hints sections | Bytecode of another ABI cannot run. Another engine build can change bytecode meaning (07-runtime §3) |
| BYTECODE | Finalize the unit's bytecode -> T0 (-> JIT in a JIT build). Drop native code. On a fact-schema mismatch, drop the hints section too | These versions describe native code only. The bytecode ABI has its own version (H2) |

- With an exact build-id match, step 4 fails in practice only on CPU features. The per-contract versions stay: they keep the mutation tests exact, and a later ADR can relax step 3 to "same bytecode ABI -> BYTECODE" without a format change.

**S5. Rules.**
- Zero text relocations. The writer rejects HeapConst; an xtask check enforces it.
- IR heap refs (RootRef, IntrinsicRef, ShapeRef, CellRef, UnitConst; H30) lower to link-table loads, RW cells or unit constants (H34).
- AOT code reaches the isolate through the reserved isolate register (x28 / r15), and its isolate's RW copy through the reserved unit-data register (x27 / r14, H35-Q9): RW accesses are register-relative, never pc-relative. pc-relative forms reach only per-process sections (text, rodata; H34-Q2).
- Link cells resolve at load. IC cells fill at first execution. Link cells are strong roots; IC cells are weak (03 §2).
- ShapeId immediates appear only for snapshot root shapes (H8). Realm values come through IntrinsicRef (H13, H22).
- Every outcome other than a full load is counted by reason and prints one warning line per unit, as Leyden does by default [R11]. `--unit-strict` turns SOURCE and BYTECODE into LOAD ERROR, as Leyden's `-XX:AOTMode=required` does.

**S6. Header v0 (as built, M0 wk4).** `crates/ep9_unit/src/header.rs`; the full contract is `layout_text()`, kept in `crates/ep9_unit/tests/golden/header-layout.txt`.

```text
   0 magic "\x89EP9UN\r\n"     8 container_version u32 (1)   12 header_size u32 (256)
  16 total_size u64            24 section_count u32 (2..9)    28 section_table_offset u32 (256)
     -- 32-byte frozen prefix --
  32 hash_alg u8 (1 = SHA-256) 33 header_flags u8 (bit 0 signed)  34 reserved [6]
  40 content_hash [32]         72 engine_build_id [16]
  88 bytecode_abi  92 value_encoding  96 object_layout  100 barrier  104 frame  108 ic
 112 deopt_meta  116 fact_schema   (u32 each)
 120 target_arch u8  121 target_abi u8  122 heap_page_log2 u8  123 reserved
 124 source_flags u32  128 cpu_required u64  136 builtin_abi u32  140..256 reserved
 section entry (32 B, at 256): kind u32, flags u32 = 0, offset u64, size u64, align_log2 u8, reserved [7]
```

| Rule | Value in container version 1 |
|---|---|
| Byte order, alignment | Little-endian; every section 8-aligned; reserved bytes, entry flags and padding are 0 (else LOAD ERROR) |
| Section table | Canonical: kinds 1-9 at most once, strictly ascending; each section starts at the 8-aligned end of the one before; the last ends at `total_size`. So a unit's content has exactly one encoding |
| Required sections | `bytecode` and `source`; a missing one is a LOAD ERROR (H5; the BYTECODE outcome and every deopt need the bytecode) |
| Content hash | SHA-256 of the whole unit with `content_hash` and the signature section's bytes set to 0. It covers the build id (unlike a CompiledUnit, whose hash excludes it), so a signature also names the engine build |
| Signature | `header_flags` bit 0 set iff a `signature` section exists (else LOAD ERROR). v0 does not verify it: D6 = no sandbox, and H35-Q2 has no scheme yet |
| Source flags | The CompiledUnit flag bits for goal, strict and TS flags (mask `0xf07`), same validity rules; invalid flags are a LOAD ERROR |
| Version 0 | "No native form yet". A unit with a `text` section gives BYTECODE while any native contract (value encoding, object layout, barrier, frame, IC, deopt metadata, builtin ABI) is 0. Today `DEOPT_META_VERSION` = 0 (`crates/ep9_vm/src/versions.rs`), so no unit runs native code |
| Target | `target_arch` 1 x86_64, 2 aarch64; `target_abi` 1 linux, 2 apple, 3 windows; 0 = unknown, never matches |
| CPU features | x86-64 bits 0-10 (sse3, ssse3, sse4.1, sse4.2, popcnt, avx, avx2, fma, bmi1, bmi2, lzcnt); AArch64 bits 32-36 (lse, jsconv, dotprod, fp16, rcpc). Host must have every required bit; a bit not listed counts as missing (H35-Q5) |
| Build id | `ep9_unit::ENGINE_BUILD_ID` = the first 16 bytes of SHA-256 over the git commit (`unknown` without git), `rustc -vV`, the target and `ep9_unit`'s own Cargo features; `EP9_BUILD_ID=<32 hex digits>` at build time overrides it (reproducible builds). `crates/ep9_unit/build.rs` computes it and reruns when HEAD moves (H35-Q8). `ep9_frontend` still writes 16 zero bytes into CompiledUnits (`crates/ep9_frontend/src/bytecodegen/mod.rs:91`), so the bytecode corpus digest does not depend on the build |
| Container version | Any change to the layout text needs a bump; `tests/golden/container-history.txt` pins the SHA-256 of the text per version. The text includes the CPU bits and target codes, so even adding one needs a bump, and a bump makes every older unit a LOAD ERROR. That is free while no unit exists; before the A3 freeze, an append-only history for these code tables can replace it (an older engine already treats an unknown bit as missing) |

`check(bytes, &HostInfo) -> Outcome` is a pure function; `verdict` also gives the reason (S5 counts by reason). `HostInfo::current()` reads `ABI_VERSION`, `encoding::VERSION`, `BARRIER_VERSION`, the `ep9_vm::versions` constants, `FACT_SCHEMA_VERSION`, `PAGE_SIZE`, the compile target and the CPU features detected at run time. The `aot` feature holds an empty module until the A2 writer; tests build units with a test-only encoder.

## Consequences

- **Gains.** Isolate-independent code; text shared per process, RW data per isolate. iOS-ready `.o` form. A stale unit degrades to bytecode or source, or ends in a clean load error, never in a crash.
- **Costs.** Deopt metadata sets much of the unit size (12.5-17.1 deopt points per KB in TurboFan code [R14]). Each non-root shape guard pays a cell load through the unit-data register (H8). Each unit function pays the unit-data entry (two loads) and a save and restore of x27 / r14 (H35-Q9).
- **Follow-ups.** H34 relocation kinds; H29 PI metadata; A2 writer/loader with the `object` crate (exact `=` pin chosen at A2).

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R14] | Wasmtime refuses precompiled code with relocations outside `.debug_*`; V8 embedded builtins use a root register; iOS needs static objects in the signed app | [14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R11] | JEP 544: AOT code needs the same CPU features and GC barriers, or it is rejected | [11-dual-mode-vms.md](../research/11-dual-mode-vms.md) |
| [R17] | RW cells filled at first execution replace deterministic ShapeIds (Static Hermes `{clazz, slot}`) | [17-aot-refutation.md](../research/17-aot-refutation.md) |

## Validation

- **SP-Unit** ([09 §4](../plan/09-milestones.md#4-spikes), M3-P1; moved from M0 month 2 by M2-D21-FAIL): the 11 Spike B opcodes (09 §4 said 10; Spike B adds AddSmi) built as a unit blob load, call, unwind, GC-scan and fall back on a version mismatch, on 4 targets. On fail it reopens H34 and H35. M0 exit item 11. **Built in M3-P1** (`spikes/spike-b/src/unit/`, `bash spikes/spike-b/gate.sh unit`, checks (a)-(g); [data](../measurements/2026-10-23-sp-unit.md)): it passes on mac a64, linux a64 (Docker), linux x64 (the sandbox) and windows-x64 (CI probe run 39886596197, b92754d plus one probe commit: "unit: PASS on x86_64 (windows, ...)"; decision M3-P1-SPUNIT); since then `ci.yml` runs the unit gate on all 4 runners, and M3-P2 WIN-STATIC-2 adds the Win64 unwind check of unit text (`spikes/spike-b/tests/win64_unit.rs`). The verdict row is the orchestrator's.
- Unit golden files: the header struct and container spec v0 match committed golden files (M0 exit; [08 §7](../plan/08-repo.md#7-dependency-guards-and-xtask-checks)). **Built M0 wk4** (`cargo test -p ep9_unit`): `header-v0.hex`, `header-layout.txt` and `container-history.txt` (a layout change without a container version bump fails); every header field mutated bit by bit and with random values ends in its listed outcome, and without a recomputed hash every change is a LOAD ERROR; every section-entry mutation is a LOAD ERROR or another canonical unit; 10^5 random flips, truncations and extensions never panic; a test-encoded unit gives LOAD on this build, SOURCE with another bytecode ABI, and BYTECODE with a text section (deopt version 0).
- A2 gate: blob and `.o` on 4 targets; zero text relocations; 100% of header mutations end in their S3 outcome. Never a crash, never a wrong result.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H35-Q1 | Byte layout of the header: field sizes, order, endianness, alignment | M0 wk4 header struct | **Filled (M0 wk4, W4-AOTU-HEADER).** S6: 256 bytes, little-endian, fixed offsets, 32-byte frozen prefix, canonical 8-aligned section table |
| H35-Q2 | Content-hash algorithm and signature scheme (D6 decides if a signature is required) | M0 exit | **Filled 2026-10-23 (M3-P1 SP-UNIT, V8-A16).** Content hash: SHA-256 (`HASH_SHA256` = 1, `crates/ep9_unit/src/header.rs:45-46`; the Content hash rule row in S6). Signature: the slot stays reserved (section 9 `Signature`, `crates/ep9_unit/src/codes.rs:244-245`; `header_flags` bit 0; the Signature rule row in S6), and v0 verifies nothing. No signature scheme in M0: D6 = no sandbox in v1 (decided 2026-10-20), so a signature stays optional. The scheme is an A2/A3 item |
| H35-Q3 | Section names in Mach-O, ELF and COFF for the `.o` form | A2 | Decide at A2 |
| H35-Q4 | One version per contract (as listed) or one combined ABI version | M0 wk4 | **Filled (M0 wk4, W4-AOTU-HEADER).** One u32 per contract: the 05 §8 list plus the builtin ABI (H20). Version 0 = no native form yet |
| H35-Q5 | How CPU features are encoded and matched (exact vs subset) | M0 wk4 | **Filled (M0 wk4, W4-AOTU-CPU).** u64 required-feature bitset; the host must have every required bit; a bit not defined counts as missing |
| H35-Q6 | Is page size (H09-Q4) part of the object-layout version? | M0 wk4 | **Filled (M0 wk4, W4-AOTU-PAGE).** No: page size is its own field (`heap_page_log2`, offset 122); a mismatch gives BYTECODE. No emitted code bakes the mask in v0 (H09-Q4) |
| H35-Q7 | S4 step 1 sends every header mismatch to the bytecode fallback. But bytecode of another ABI version, or a container with a bad magic, cannot run. Which header fields fall back to source instead? | M0 exit | **Resolved (decision H35-fallback, 2026-10-20).** Prefix (magic, container version) and hash/signature: clean load error, because no section can be trusted. Build id and bytecode ABI: source section. Native-only versions and CPU: bytecode. This differs from the earlier proposal for magic and container version, because a bad prefix hides the source section |
| H35-Q8 | Source of the engine build id | M3-P1 SP-UNIT (was SP-Unit, M0 month 2; moved by M2-D21-FAIL); filled earlier, by M2-P2-INFRA2 (2026-10-22) | **Filled 2026-10-22 (M2-P2-INFRA2): built as proposed.** The first 16 bytes of SHA-256 over the git commit (`git rev-parse HEAD`, also in a worktree; `unknown` without git), `rustc -vV`, the target and the enabled features, with an `EP9_BUILD_ID` override (32 hex digits; any other value fails the build). "Enabled features" means `ep9_unit`'s own features only: a build script cannot see the features of other crates. Text format and rerun rules: `crates/ep9_unit/build.rs`; tests in `crates/ep9_unit/src/host.rs` |
| H35-Q9 | Unit-data register (TOC style). The unit text is mapped once per process, but each isolate has its own RW copy (H22, S4 load step 6). A pc-relative access from the shared text reaches one address only, so it cannot reach a per-isolate RW copy, which S5 ("RW data pc-relative") assumes. How does AOT code reach its isolate's RW data? | M3-P1 SP-UNIT (was SP-Unit, M0 month 2; M2-P5-PLAN-D17) | **Filled 2026-10-23 (M3-P1 SP-UNIT): built as proposed, with a slot directory.** x27 (AArch64) and r14 (x86-64, every OS) hold the base of the isolate's RW copy of the unit; both are callee-saved in AAPCS64, SysV and Win64. `Cx::unit_data` (`ep9_vm::layout::CX_UNIT_DATA`) points at the isolate's unit-data directory (64 words); each unit has a slot (0-63) that is fixed when the unit is built. On entry: `x27 = [[x28 + CX_UNIT_DATA] + 8 * slot]` (x86-64: r14 from r15). Then every RW access is `[x27 + offset]`: external references, link cells (a strong root range), IC cells (a weak root range), counters. Unit code saves x27 / r14 in its frame and restores it before each exit, because (b) code is entered by a tail jump from a Rust handler. A second unit with a used slot gives BYTECODE. Rejected: one data word per process (it needs a load-time write into text or a placement fixup). Evidence: `spike-b unit` checks (c), (d), (e) and (g), [SP-Unit measurement](../measurements/2026-10-23-sp-unit.md) |

## Reopens if

- SP-Unit fails on any tier-1 target (09 §4 names H34 and H35); the A0 time box is 2 weeks (est.).
- The A2 gate fails (zero text relocations, 100% of header mutations end in their S3 outcome, unwind through AOT frames).
- D6 = sandbox (only signed or bytecode-only units may load) or D16 = yes (`.o` first; 11-decisions "Affects").
- Deployments must survive engine upgrades without a unit rebuild, and SOURCE start-up is measured too slow (A1 gates). Then relax S4 step 3 to "same bytecode ABI -> BYTECODE", backed by a cross-build bytecode-stability CI test.
- Units must outlive container-version changes: add a frozen source-locator prefix before the A3 container freeze.
- Per-section hashes are adopted (H35-Q2): a hash failure confined to native sections may then fall back to BYTECODE.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-20 | draft | Spec skeleton from 02 §6 and 05 §8 (AOT senior, M0 wk1) | ADR agent |
| 2026-10-20 | rewrite | H35-Q7 resolved: three outcomes by field (LOAD ERROR, SOURCE, BYTECODE). S3, S4, S5, Decision and Validation updated. Plan 02 §6, 05 §8, 05 §12, 07 §2-3, 09 §3, 10 §2 and RK35 aligned. A `.o` unit's LOAD ERROR is a clean startup error (critic note) | tech-gaps agent; apply agent |
| 2026-10-21 | clarify | Reopens if: the A2 gate item "header-mutation fallbacks" now uses the Validation text ("100% of header mutations end in their S3 outcome"). No check changes | hygiene agent |
| 2026-10-21 | fill | Header v0 built (`crates/ep9_unit`): Q1 layout (S6), Q4 one version per contract plus the builtin ABI, Q5 CPU subset with unknown bits missing, Q6 page size as its own field; S3 gains the builtin ABI and page size rows; the hash covers the build id; missing source or bytecode section and a signed flag without a signature section are LOAD ERRORs; version 0 = no native form; new Q8 (build id source). Golden files and mutation tests in Validation. Decisions W4-AOTU-* | P1-AOTU agent |
| 2026-10-21 | clarify | New open question H35-Q9 (unit-data register, TOC style; proposal x27 or r14), due SP-Unit. No rule changes until it is filled | P0-DOCS agent (M2-P0) |
| 2026-10-21 | clarify | 1 `path:line` reference moved to the code after M2-P0 (drift script, [I15]) | M2-P0 integration fixer |
| 2026-10-22 | fill | H35-Q8 filled: the engine build id is SHA-256 (first 16 bytes) of the commit, `rustc -vV`, the target and `ep9_unit`'s own features, with the `EP9_BUILD_ID` override; the Build id rule row says so. Unit goldens and the corpus digest keep their values (the goldens use a fixed id; CompiledUnits keep zeros) | M2-P2-INFRA2 agent |
| 2026-10-22 | clarify | H35-Q9: the decision id is M2-JIT-ABI (decisions-log; "M2-AOT-TOC" names no row), Needed by M3-P1 SP-UNIT; the Status line and the README §5 cell name the filled H35-Q8. SP-Unit is in M3-P1 (Validated by, Validation) and has 11 opcodes. The Q9 fill and the S5 rewrite stay with M3-P1 SP-UNIT. No rule or check changes (M2-P5-PLAN-D17) | P5-DOCS-SYNC agent (M2-P5) |
| 2026-10-23 | clarify | 1 `path:line` reference moved to the code after the M3-P0 merge and the M3-P0 code fixes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). No check changes | M3-P0 integration fixer |
| 2026-10-23 | rewrite | H35-Q9 filled: the unit-data register x27 / r14 with a per-isolate slot directory (`Cx::unit_data`), built by `spike-b unit`. The S5 rule "RW data pc-relative" is replaced in the Decision, in S5 and in Costs: RW data is reached through the unit-data register; pc-relative forms reach per-process sections only | M3-P1 SP-UNIT agent |
| 2026-10-23 | fill | H35-Q2 filled: SHA-256 content hash (as built); the signature slot stays reserved; no scheme in M0 (D6); the scheme is an A2/A3 item. Text only; no code, test or golden changes | M3-P1 SP-UNIT agent |
| 2026-10-23 | clarify | H35-Q8 Needed-by cell: Q8 was filled by M2-P2-INFRA2, not by SP-Unit. No check changes | M3-P1 SP-UNIT agent |
| 2026-10-23 | clarify | The Status line (and the README §5 cell) names the H35-Q2 and H35-Q9 fills and the SP-Unit build; the Validation SP-Unit item names the gate (`gate.sh unit`), the data file and the targets where it passed. No rule or check changes; the SP-Unit verdict stays with the orchestrator | M3-P1 integration fixer (from the SP-UNIT report) |
| 2026-10-23 | clarify | New `## Review record` section (README §3.1; B4) with rows 1 and 4 by M3-P1 ACCEPT-5 step 2, over the H35-Q2 and H35-Q9 fills, the S5 rewrite and the H35-Q8 Needed-by cell. Rows 2 and 3 are placeholders for the orchestrator's Codex run and M3-P2 ACCEPT-6. Row 1 lists 6 notes for the owner; no text outside the record changes, and no check changes | M3-P1 ACCEPT-5 (step 2) |
| 2026-10-23 | clarify | H35-Q8 Needed-by cell: SP-Unit is in M3-P1 (moved by M2-D21-FAIL), not M0 month 2; the cell now says "M3-P1 SP-UNIT (was SP-Unit, M0 month 2; moved by M2-D21-FAIL)", as the H35-Q9 cell does. Review record row 3 and a row-4 addendum (M3-P2 ACCEPT-6, V1). Text only; no check changes | M3-P2 ACCEPT-6 |
| 2026-10-23 | clarify | Validation, SP-Unit item: windows-x64 is no longer open; CI probe run 39886596197 passed the unit gate on windows-x64 (decision M3-P1-SPUNIT), `ci.yml` runs the gate on all 4 runners, and M3-P2 WIN-STATIC-2 adds `tests/win64_unit.rs` (WIN-STATIC-2 fixer item 2). The "Missing: windows-x64" text of review row 4 (checked at 80b7986) stays: it records that check; the M3-P2 row-4 addendum above it states the probe result. Text only; no check changes | M3-P2 integration fixer (docs) |
| 2026-10-24 | clarify | Review record: row 2 filled from the Codex texts; it names the runs that read H35 (the old placeholder named the M3-P1 B4 slice, which had no H35 diff) and says that no Codex run gave a verdict on the 9acc400 text (the SP-UNIT slice stopped at the content filter; row 1 is the Claude review). A row-3 addendum by the M3-P2 row-3 verifier (at df87c4a; nothing to sort). No text outside the record changes; no check changes | M3-P2 records writer |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | M3-P1 ACCEPT-5 step 2 (did not write the ADR or any Change-log row before the 2026-10-23 review row) | Checked at 80b7986 (the M3-P1 merge 94ed243 plus the M3-P1 fixer merges; the SP-UNIT code is 9acc400 plus the SPUNIT-1 fix 8f19770). Decision = the 02 §6 H35 row: both say "Isolate register + unit-data register for the per-isolate RW data (H35-Q9) + link table" (9acc400 changed 02 §6, the 04 §5 LinkSlot and IcCell rows, 05 §8 and 06 §2.1 in the same commit). S6 = the golden `header-layout.txt` (each offset, size, on-change outcome, section kind and fixed value; history row 1 `748e44cc…`); the S6 "Version 0" row is true (`DEOPT_META_VERSION` = 0, versions.rs:21 at 80b7986). H35-Q9 fill against the code: true. (1) Entry = two loads: x27 = [[x28 + CX_UNIT_DATA] + 8 * slot] (`load_unit_data`, a64.rs:101-111 at 80b7986), r14 the same from r15 (x64.rs:60-71). (2) The frame saves x27 (`str x27, [sp, #32]`) or r14 (`push r14`), and the one common exit `leave` restores it before `br x16` or the exit jump (a64.rs:341-344, x64.rs:327-339); the stack-overflow, slow-path throw and unwind paths all go to `leave`. (3) Unit code reads and writes RW words only as [x27 / r14 + offset]: external reference 0 (the runtime table, `call_table`), IC cells and counters; check (c) reads the link cells through the unit's cell reader. (4) `SLOTS` = 64 (emit.rs:35); the slot is a build input (`UnitSpec::slot`); the directory is per isolate (`UnitHost::dir`). (5) Link cells are a strong root range, IC cells a weak one (load.rs:602-619). (6) A used slot gives BYTECODE: in the process (load.rs:126-130; check (g) "slot conflict") and in the isolate (load.rs:510-515). (7) x27 and r14 are callee-saved in AAPCS64, SysV and Win64; the x86-64 unit code is `extern "sysv64"` on every OS. S5 rewrite: on AArch64 the text scan rejects `adrp`, `bl` and each pc-relative form that leaves its function (scan.rs); check (a) finds 0 relocations in 4 units (AArch64 and x86-64). Costs: true (two entry loads, one save and one restore per function). H35-Q2 fill: true: `HASH_SHA256` = 1 (header.rs:45-46), section 9 `Signature` (codes.rs:244-245), `header_flags` bit 0; `check` only compares the flag with the section and hashes the unit with the signature bytes set to 0 (check.rs:197-202), so v0 verifies no signature. H35-Q8 Needed-by cell: true (the 2026-10-22 fill row of M2-P2-INFRA2). Notes, not changed here (this role owns only this section): N1. S4 gives no outcome for a failure in step 6 (an unknown external-reference kind or intrinsic index). Since 8f19770 the spike gives LOAD ERROR before any section runs (load.rs:497-501; check (g) has 2 link cases, each followed by gc()). The A2 loader needs a written rule (an Open question or an S4 line; owner AOT senior). N2. The Q2 fill sends the signature scheme to "A2/A3", but no Open-question row tracks it. N3 (the orchestrator's open item X-SPU-STRICT): proven. The spike's SOURCE path compiles the source section with the default script options and does not read `source_flags` (load.rs:435-447). A probe (outside the repo): a sloppy unit with the STRICT bit set in its header and another build id gives SOURCE, and `this === undefined` in a plain function call prints false (node v24 in strict code: true). No unit that the spike builds gets there, because `build` takes the flags from the same compile (build.rs:90-93). So it is a requirement on the A2 loader (the S4 SOURCE row), not a defect in the H35 text (category (c) for M3-P1). N4. The 04 §5 Runtime(fn) row still says that unit code calls through the isolate table; the unit code loads the table from RW external-reference word 0 (decision SPUNIT-RT proposes the 04 §5 change at A2). N5. `Cx::unit_data` and `CX_UNIT_DATA` exist only with the feature `spike-native`; the field doc (interp/mod.rs:166-167) still says "reserved for SP-UNIT ... Nothing reads it in M3-P0". N6. The x86-64 text scan uses the window rule only (no decoder), so the S5 pc-relative rule is checked by decode on AArch64 only. Change-log kinds: agree. The 2026-10-23 rewrite (Q9, S5) changes a Decision statement and an S5 rule; it is pre-freeze churn (H35 freezes at the A3 gate), so G8 does not count it. The Q2 fill, the Q8 clarify and the fixer's Status and Validation clarify: agree. The older rows: agree; "draft" (2026-10-20) is the first text, not a §3.3 kind. Open questions due in M0: none open (Q2 and Q9 filled 2026-10-23; Q1 and Q4-Q8 filled or resolved before; Q3 is A2) | branch m0m3/p1-fix-docs; read at 80b7986; probe outside the repo: scratchpad m0m3/p1/a5s2logs/probe_strict.rs.txt |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | Filled 2026-10-24 from the Codex texts. All runs: `/codex:adversarial-review --model gpt-6.1-sol`, in the background. (1) The M3-P1 B4 docs slice (base 2ba3aa4), which the old placeholder named: it had no H35 diff (row 3). (2) The M3-P1 SP-UNIT code slice: it found SPUNIT-1 (fixed, 8f19770), then stopped at the content filter, so no Codex run gave a verdict on the 9acc400 text (the H35-Q9 fill and the S5 rewrite). The plan gives SP-UNIT JIT/AOT code to the Claude reviewer when Codex refuses; row 1 (M3-P1 ACCEPT-5 step 2) is that review of the text. (3) the M3-P1 fixer docs slice (base 94ed243, head b92754d): needs-attention, 5 findings (X-FD1 to X-FD5); it read rows 1 and 4 and the fixer's Status and Q8 rows; none on H35, and Codex wrote that the H35 addenda match the unit-register code. (4) M3-P2 round 1, the ACCEPT-6 docs slice (base cbae45d, head fb6f06f) (row 3, the row-4 addendum and the H35-Q8 V1 clarify): needs-attention, 1 finding, X-ACC1 on H12; none on H35. (5) M3-P2 round 2, the docs slice (base 1edad65, head 9052884 = the ADR text of e4b7917) (the fixer's SP-Unit windows-x64 clarify): needs-attention, 2 findings, R3-4 on H15 and R3-5 on H22; none on H35. Open finding on H35: none | Codex texts outside the repo: scratchpad m0m3/p1/codex-b4.out, codex-spunit.out, codex-fix-docs.out; m0m3/p2/codex/codex-accept-6.out, r2-docs.out; row written by the M3-P2 records writer from those texts |
| 3 Verifier | M3-P2 ACCEPT-6 (did not write H35 text, rows 1-2 or any Change-log row before this row and its V1 clarify) | Checked at cbae45d (`spikes/` and `crates/ep9_unit` have no change since 80b7986, the row-1 head). Row 1 facts: they still hold; N1-N6 still hold at cbae45d (for example N1: load.rs:497-501 at cbae45d puts the parse and the link plan before the first run; N3: `run_source`, load.rs:435-447 at cbae45d, does not read `source_flags`; N5: the field doc interp/mod.rs:166-167 at cbae45d still says "Nothing reads it in M3-P0"). They stay notes for the owner (AOT senior; N5 for a VM role). Row 2 (Codex) at the time of this row: a placeholder, and its text names the wrong run: the M3-P1 B4 docs slice (codex-b4.out, base 2ba3aa4) had no H35 diff. The runs that read H35: the fix-docs slice (codex-fix-docs.out, 94ed243..b92754d) read the row-1/row-4 addenda and the fixer's Status and Q8 rows and gave no H35 finding ("The H08/H13/H35 addenda match the unit-register code"); the SP-UNIT code slice (codex-spunit.out) found SPUNIT-1 (fixed, 8f19770) and was then stopped by the content filter, so no Codex run gave a verdict on the 9acc400 text (the H35-Q9 fill and the S5 rewrite). Row 2 must say so; the plan gives SP-UNIT JIT/AOT code to the Claude reviewer when Codex refuses (row 1 is that review). V1 (clarify, done in this commit; a fact, not a choice): the H35-Q8 Needed-by cell said "SP-Unit (M0 month 2)", but M2-D21-FAIL moved SP-Unit to M3-P1, as the H35-Q9 cell says; it now says "M3-P1 SP-UNIT (was SP-Unit, M0 month 2; moved by M2-D21-FAIL); filled earlier, by M2-P2-INFRA2 (2026-10-22)". Change-log kinds: agree (row 1, and the ACCEPT-5 record row: clarify). Open questions due in M0: none open. Fix by SP-Unit (M3-P1): met on 4 targets (row 4 addendum). Before Accept: row 2 (orchestrator) | read and runs at cbae45d; Codex logs outside the repo: scratchpad m0m3/p1/codex-fix-docs.out, codex-spunit.out |
| 4 Data (M3-P2 addendum) | M3-P2 ACCEPT-6 | At cbae45d on mac a64: `bash spikes/spike-b/gate.sh unit`: SPIKEB(unit) PASS (unit/tests 8 pass, unit/lib-tests 5 pass, checks (a)-(g) PASS, "unit: PASS on aarch64 (macos, bytecode ABI 5, FRAME 2, OBJECT_LAYOUT 3, unit container 1)"); `cargo test -p ep9_unit --locked`: 28 pass; `cargo test -p ep9_vm --features spike-native --locked native`: 35 pass. The x86-64 run that row 4 asked for at the M3-P1 fixer head is done: at e86ecae (the code tree of cbae45d), the orchestrator's local CI x86 sandbox step `spike-b-unit-gate` PASS ("unit: PASS on x86_64 (linux, ...)") and Docker linux a64 SPIKEB(unit) PASS. windows-x64: the CI probe run 39886596197 (b92754d plus one probe commit; it has SPUNIT-1; after it only realm code in `ep9_vm` changed) SPIKEB(unit) PASS ("unit: PASS on x86_64 (windows, ...)"), and the `ci.yml` unit gate runs on all 4 runners since then. Missing: the A2 gate (A2) | local run, mac a64, cbae45d; logs outside the repo: scratchpad m0m3/p2/a6logs/gate-unit.log, unit.log, vm-native.log; scratchpad ci/local/m3-p1-final/x86-sandbox.log, docker-arm64.log; scratchpad m0m3/p1/probe-win-spikes.log |
| 4 Data | from the "Validated by" line | SP-Unit: `bash spikes/spike-b/gate.sh unit` at 80b7986 on mac a64: SPIKEB(unit) PASS. unit/tests (`tests/unit.rs`, 8 pass), unit/lib-tests (5 pass: the scan and the code generators), and the release `spike-b unit`: checks (a)-(g) PASS, no SKIP line. (a) 4 units (AArch64 and x86-64 text) with 0 relocation findings; (b) output = T0 with the default, eager and GC-stress configs, and the text is mapped once for 3 isolates; (c)-(e) the RW copy through x27 (strong link cells, weak IC cell), 2 isolates on 2 threads with one text, 2 units in both load orders; (f) unwind self-test mode per-fde, a native walk and an exception through unit frames, termination 208.8 ms after the start (request at 200 ms); (g) 21 cases, each with its S3 outcome and output = T0, with the 2 link-failure cases (LOAD ERROR, then gc()). `cargo test -p ep9_unit --locked`: 28 pass (lib 2, build_watch 3, check 12, fuzz 2, header 9: the header and layout goldens, the container history, the mutation tests). `cargo test -p ep9_vm --features spike-native --locked native`: 35 pass (with the runtime-table golden, its negative control and the unit-data word test). Other targets, not run again here: linux a64 Docker (SP-UNIT role, 9acc400) and linux x64 (the orchestrator's sandbox request sp-unit-1, 9acc400): SPIKEB(unit) PASS. Since 9acc400 the code changed in load.rs, format.rs and checks.rs (SPUNIT-1) and in the ep9_vm spike-native call paths (REALMS-1), so an x86-64 run at the M3-P1 fixer head is still to do. Missing: windows-x64 (a CI probe or M3-P2 WIN-STATIC-2); the A2 gate (A2). The SP-Unit verdict row is the orchestrator's | local run, mac a64, 80b7986, `EP9_HASH_SEED=1`; logs outside the repo: scratchpad m0m3/p1/a5s2logs/gate-unit.log, unit.log, vm-native.log |
| 3 Verifier (M3-P2 addendum: Codex findings on the M3-P2 edits) | M3-P2 row-3 verifier (wrote no H35 text, no review row, no Codex text and no fix) | Checked at df87c4a. Codex runs that hold the M3-P2 H35 edits: round 1, the ACCEPT-6 docs slice (base cbae45d; row 3, the row-4 addendum and the H35-Q8 V1 clarify): no finding on H35. Round 2 docs slice (base 1edad65, head 9052884; the fixer's SP-Unit windows-x64 clarify): no finding on H35. Findings to sort: none. `crates/ep9_unit` and `spikes/spike-b/src` did not change from cbae45d to df87c4a; `spikes/spike-b/tests/win64_unit.rs` exists, as the clarify says. Open (a)/(b): none | read at df87c4a; Codex texts: scratchpad m0m3/p2/codex (codex-accept-6.out, r2-docs.out) |
