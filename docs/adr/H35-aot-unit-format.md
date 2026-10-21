# H35: AOT unit format (container spec skeleton)

| Field | Value |
|---|---|
| Status | Draft, spec skeleton (2026-10-20). Header v0, section table and load check (S4 steps 1-4) built in `crates/ep9_unit` (M0 wk4, S6); writer and loader A2 |
| Fix by | Spec v0 + header struct + golden test by M0 exit; writer/loader A2; frozen at A3 gate |
| Freeze point | A3 gate |
| Origin | R16-D6 + R17; plan status N (new) |
| Crates | `ep9_unit` (container, header, verify, loader; writer under `aot`), `ep9_masm` (unit finalize), `ep9_platform` (mapping, unwind registration) |
| Owner (plan role) | AOT senior |
| Validated by | SP-Unit (M0 month 2, inside Spike B); A2 gate |
| Plan sources | [02 §6 H35](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [05 §8](../plan/05-aot.md#8-aot-unit-format-h35), [04 §5](../plan/04-execution.md#5-jit-infrastructure-and-the-relocatable-macroassembler), [03 §6](../plan/03-gc.md#6-snapshot-interaction) |

## Context

- Units carry AOT code (baseline-AOT, optimized-AOT) and the bytecode it came from. One unit text must serve every isolate in a process.
- iOS needs code inside the signed app (D16). Wasmtime refuses text relocations. Leyden rejects mismatched code [R14][R11].
- A unit is trusted code, like a native addon (D6). A stale or mismatched unit must never crash (RK35).
- This file is the M0 wk1 skeleton. The header struct and golden test come in M0 wk4; spec v0 is due at M0 exit (exit item 10).

## Decision

Zero text relocations. Isolate register + pc-relative RW data + link table. Link and IC cells resolved at load or first execution. ShapeId immediates only for snapshot root shapes. Header: versions, CPU features, content hash (+ optional signature). `.o` or `.ep9u` blob. Mismatch → by field (S3): native-only versions and CPU → the unit's bytecode; engine build id or bytecode ABI → the unit's source section; bad prefix, hash or signature → clean load error.

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
- AOT code reaches the isolate through a reserved isolate register, and its RW data pc-relative.
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
| Build id | 16 zero bytes until SP-Unit (H35-Q8), as `ep9_frontend` writes into CompiledUnits today (`crates/ep9_frontend/src/bytecodegen/mod.rs:82`) |
| Container version | Any change to the layout text needs a bump; `tests/golden/container-history.txt` pins the SHA-256 of the text per version. The text includes the CPU bits and target codes, so even adding one needs a bump, and a bump makes every older unit a LOAD ERROR. That is free while no unit exists; before the A3 freeze, an append-only history for these code tables can replace it (an older engine already treats an unknown bit as missing) |

`check(bytes, &HostInfo) -> Outcome` is a pure function; `verdict` also gives the reason (S5 counts by reason). `HostInfo::current()` reads `ABI_VERSION`, `encoding::VERSION`, `BARRIER_VERSION`, the `ep9_vm::versions` constants, `FACT_SCHEMA_VERSION`, `PAGE_SIZE`, the compile target and the CPU features detected at run time. The `aot` feature holds an empty module until the A2 writer; tests build units with a test-only encoder.

## Consequences

- **Gains.** Isolate-independent code; text shared per process, RW data per isolate. iOS-ready `.o` form. A stale unit degrades to bytecode or source, or ends in a clean load error, never in a crash.
- **Costs.** Deopt metadata sets much of the unit size (12.5-17.1 deopt points per KB in TurboFan code [R14]). Each non-root shape guard pays a pc-relative cell load (H8).
- **Follow-ups.** H34 relocation kinds; H29 PI metadata; A2 writer/loader with the `object` crate (exact `=` pin chosen at A2).

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R14] | Wasmtime refuses precompiled code with relocations outside `.debug_*`; V8 embedded builtins use a root register; iOS needs static objects in the signed app | [14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R11] | JEP 544: AOT code needs the same CPU features and GC barriers, or it is rejected | [11-dual-mode-vms.md](../research/11-dual-mode-vms.md) |
| [R17] | RW cells filled at first execution replace deterministic ShapeIds (Static Hermes `{clazz, slot}`) | [17-aot-refutation.md](../research/17-aot-refutation.md) |

## Validation

- **SP-Unit** ([09 §4](../plan/09-milestones.md#4-spikes), M0 month 2): the Spike B 10 opcodes built as a unit blob load, call, unwind, GC-scan and fall back on a version mismatch, on 4 targets. On fail it reopens H34 and H35. M0 exit item 11.
- Unit golden files: the header struct and container spec v0 match committed golden files (M0 exit; [08 §7](../plan/08-repo.md#7-dependency-guards-and-xtask-checks)). **Built M0 wk4** (`cargo test -p ep9_unit`): `header-v0.hex`, `header-layout.txt` and `container-history.txt` (a layout change without a container version bump fails); every header field mutated bit by bit and with random values ends in its listed outcome, and without a recomputed hash every change is a LOAD ERROR; every section-entry mutation is a LOAD ERROR or another canonical unit; 10^5 random flips, truncations and extensions never panic; a test-encoded unit gives LOAD on this build, SOURCE with another bytecode ABI, and BYTECODE with a text section (deopt version 0).
- A2 gate: blob and `.o` on 4 targets; zero text relocations; 100% of header mutations end in their S3 outcome. Never a crash, never a wrong result.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H35-Q1 | Byte layout of the header: field sizes, order, endianness, alignment | M0 wk4 header struct | **Filled (M0 wk4, W4-AOTU-HEADER).** S6: 256 bytes, little-endian, fixed offsets, 32-byte frozen prefix, canonical 8-aligned section table |
| H35-Q2 | Content-hash algorithm and signature scheme (D6 decides if a signature is required) | M0 exit | Hash now; signature slot reserved. D6 = no sandbox in v1 (decided 2026-10-20), so the signature stays optional |
| H35-Q3 | Section names in Mach-O, ELF and COFF for the `.o` form | A2 | Decide at A2 |
| H35-Q4 | One version per contract (as listed) or one combined ABI version | M0 wk4 | **Filled (M0 wk4, W4-AOTU-HEADER).** One u32 per contract: the 05 §8 list plus the builtin ABI (H20). Version 0 = no native form yet |
| H35-Q5 | How CPU features are encoded and matched (exact vs subset) | M0 wk4 | **Filled (M0 wk4, W4-AOTU-CPU).** u64 required-feature bitset; the host must have every required bit; a bit not defined counts as missing |
| H35-Q6 | Is page size (H09-Q4) part of the object-layout version? | M0 wk4 | **Filled (M0 wk4, W4-AOTU-PAGE).** No: page size is its own field (`heap_page_log2`, offset 122); a mismatch gives BYTECODE. No emitted code bakes the mask in v0 (H09-Q4) |
| H35-Q7 | S4 step 1 sends every header mismatch to the bytecode fallback. But bytecode of another ABI version, or a container with a bad magic, cannot run. Which header fields fall back to source instead? | M0 exit | **Resolved (decision H35-fallback, 2026-10-20).** Prefix (magic, container version) and hash/signature: clean load error, because no section can be trusted. Build id and bytecode ABI: source section. Native-only versions and CPU: bytecode. This differs from the earlier proposal for magic and container version, because a bad prefix hides the source section |
| H35-Q8 | Source of the engine build id | SP-Unit (M0 month 2) | SHA-256 of (git commit, `rustc -vV`, target, enabled features), first 16 bytes, with an `EP9_BUILD_ID` override for reproducible builds. Until then the id is 16 zero bytes (`ep9_unit::ENGINE_BUILD_ID`) |
| H35-Q9 | Unit-data register (TOC style). The unit text is mapped once per process, but each isolate has its own RW copy (H22, S4 load step 6). A pc-relative access from the shared text reaches one address only, so it cannot reach a per-isolate RW copy, which S5 ("RW data pc-relative") assumes. How does AOT code reach its isolate's RW data? | SP-Unit (M0 month 2) | A reserved unit-data register, like a TOC pointer: x27 on aarch64, r14 on x86-64 (every OS), next to the isolate register (x28 or r15, M2-JIT-ABI). Both are callee-saved in AAPCS64, SysV and Win64. AOT code loads it on entry from the callee's per-isolate RW base, and RW accesses are register-relative. Filling Q9 rewrites the S5 rule (M2-AOT-TOC) |

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
| 2026-10-22 | clarify | New open question H35-Q9 (unit-data register, TOC style; proposal x27 or r14), due SP-Unit. No rule changes until it is filled | P0-DOCS agent (M2-P0) |
