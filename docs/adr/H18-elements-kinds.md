# H18: Elements kinds

| Field | Value |
|---|---|
| Status | Draft (2026-10-21; v0 notes). Elements kinds are not built yet |
| Fix by | M1a |
| Freeze point | M1a |
| Origin | v1#14; plan status U |
| Crates | `ep9_vm` (ARRAY and ELEMENTS cells, element paths in `object.rs` and `property.rs`); later `ep9_baseline`, `ep9_opt` (kind checks), `ep9_facts` (elements kind in LiteralFacts, H16) |
| Owner (plan role) | VM lead |
| Validated by | `ep9_vm` array tests; test262 `built-ins/Array` and `language/` array files; later the H16 FactSnapshot elements kind (SP-Facts, M1) |
| Plan sources | [02 §6 H18](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [06 §2.3](../plan/06-object-model-and-spec.md#23-elements-keys-realms-contexts-symbols-h18-unchanged) |

## Context

- Goal: no tag checks in numeric arrays [R06]. JIT and AOT code then load an int32 or a raw f64 with one kind check per array, not one tag check per element.
- G6: a sparse or huge index must not cost memory in proportion to the index.
- Depends on: H6 (hole value 0x22, NaN purification), H8 and H17 (header, the elements slot). Depended on by: H16 (elements kind in feedback and FactSnapshot), H29 (deopt on a kind change), H30 (facts).

## Decision

PACKED/HOLEY × INT32/DOUBLE/ELEMENTS, one-way. DOUBLE stores raw f64 with a hole NaN. Dictionary for sparse arrays (02 §6). Separate kinds for dictionary, frozen/sealed, typed array, arguments and string wrapper (06 §2.3).

```text
 PACKED_INT32 ──> PACKED_DOUBLE ──> PACKED_ELEMENTS
      │                │                  │
      v                v                  v
 HOLEY_INT32  ──> HOLEY_DOUBLE  ──> HOLEY_ELEMENTS ──> DICTIONARY
 (transitions go right or down only; never back)
```

**v0 notes (as built, `dabc38a`; checked again at the P3 merge, `479b1cd`).** v0 has one elements form for every array.

| Item | v0 behavior | Where |
|---|---|---|
| Storage | ARRAY cell plus one ELEMENTS cell of Values; a hole is the H6 empty value 0x22 | `crates/ep9_vm/src/object.rs:21`, `crates/ep9_vm/src/object.rs:34`; `new_elements` (`crates/ep9_vm/src/object.rs:847`) |
| Dense limit | 2^26 elements (`arr::MAX_LENGTH`) | `crates/ep9_vm/src/object.rs:285-286` |
| Growth | a store at index i makes capacity max(i + 1, 2 × capacity, 4) | `array_set` (`crates/ep9_vm/src/object.rs:951`) |
| High dense index | `a[2**26 - 1] = 1` allocates 2^26 slots: 540,606,464 B peak RSS (empty script: 3,473,408 B) | measured 2026-10-21 at `dabc38a`, release `ep9`, M5 Max (not measured again; the growth rule is unchanged) |
| Index at or above 2^26 | stored as a string-keyed property, so the array moves to dictionary mode (an index key never enters a shape, OM-10); `length` does not change (ECMA-262: index + 1) | `crates/ep9_vm/src/property.rs:370-374` falls through |
| `length` above 2^26 | RangeError "Invalid array length" (ECMA-262 allows up to 2^32 - 1) | `crates/ep9_vm/src/property.rs:361-367` |
| `delete a[i]` | writes a hole | `crates/ep9_vm/src/property.rs:743-750` |
| Index keys on other objects | string keys (an atom with the index cached; no number keys, W4-NO-NUMBER-KEYS); the first one moves the object to dictionary mode (OM-10) | `shape::add`; test `om10_an_index_key_on_a_non_array_makes_a_dictionary` |
| Element attributes | an accessor on an array index, an element whose attributes are not {w, e, c}, and a non-writable `length` throw "not implemented in ep9 v0" until element storage has attributes; an accessor descriptor for `length` returns false (W4-ACC-ELEM) | `property::define_own_property` |
| Index keys on a prototype | while no prototype has an index key (the sticky `shape::proto_index_keys` flag, H17), `[[Set]]` and `push` add a missing element with no chain walk; after that they walk the chain, so an inherited setter runs (W4-P3FIX-ELEM-FLAG) | `property::array_store` |

## Rules

| Rule | Check | From |
|---|---|---|
| v0: every element read tests for the hole value | `array_get` | M0 wk3 |
| Index keys never enter shape tables | `om10_an_index_key_on_a_non_array_makes_a_dictionary` (`tests/objects.rs`, one test per OM-10 trigger) | M0 wk4 |
| Kind transitions are one-way | unit tests per transition; a kind never returns to a narrower kind | M1a |
| A DOUBLE element is a raw f64; the hole is one NaN pattern that NaN purification (H6) never makes | store and load tests with every NaN pattern | M1a |
| A store far past the end goes to dictionary elements, not to a dense allocation | memory test: one high index keeps RSS flat | M1a |

## Consequences

- **Gains (target).** No tag check per element in numeric arrays. A kind is one byte that ICs and FactSnapshot can carry.
- **Costs (target).** More IC states (one per kind) and kind transitions in every store path. A NaN payload is reserved for the hole.
- **Costs (v0).** Holes and every element cost 8 bytes. A high index allocates the dense range. Indices at or above 2^26 break `length`. Files that use indices near 2^32 cannot pass until dictionary elements exist (59 `built-ins/Array` files name 4294967294 or 4294967295).
- **Follow-ups.** Dictionary elements remove the 2^26 limit (H18-Q4). H17 decides where non-array objects keep elements (H18-Q3).

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R06] | V8 stores elements apart from properties, in 21 elements kinds: a one-way lattice of PACKED/HOLEY over SMI, DOUBLE and ELEMENTS | [06-execution-tiers.md](../research/06-execution-tiers.md) |
| [M] | v0 limits and the high-index RSS above | This ADR, measured 2026-10-21 |

## Validation

- M1a: unit tests per kind and transition; test262 `built-ins/Array` with no new failure; the elements kind appears in H16 feedback.
- SP-Facts (M1): the FactSnapshot elements kind round-trips.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H18-Q1 | The hole NaN bit pattern for DOUBLE, next to the canonical NaN (H06-Q3) | M1a | A NaN payload that purification never makes. **Proposal 2026-10-27 (M4-P2 ADR-A0, Draft; the month-5 ELEMENTS-KINDS input):** the hole is 0xFFF7_FFFF_FFF7_FFFF (V8's hole NaN, external). Every DOUBLE-elements store, in every tier, makes a NaN canonical first (`PURE_NAN_BITS` 0x7FF8_0000_0000_0000, H06-Q3, `crates/ep9_value/src/value.rs:39`), so it never writes the hole; boxing already does this for T0, but an unboxed f64 from JIT or unit code (for example a `Float64Array` element whose bits are the hole pattern) must be made canonical by the store itself; a load compares all 64 bits. The DOUBLE store is a raw f64 cell, a new kind with no Values (OBJECT_LAYOUT 5 in M5-P0, V9-D39), so the GC never reads its words (H21 P1) |
| H18-Q2 | The sparse rule: when does a store go to dictionary elements? | M1a | A gap rule on (index − length) and a fill ratio, as V8. **Proposal 2026-10-27 (M4-P2 ADR-A0, Draft):** a store at index i goes to DICTIONARY when i − capacity ≥ 1,024 (V8's `kMaxGap`, external; est.), or when the new capacity would pass the dense limit 2^26; a `length` set past the dense limit also goes to DICTIONARY. The move is one-way (the Decision lattice); no fill-ratio rule moves an array back to a dense kind in v1. Dictionary entries hold (value or ACCESSOR_PAIR, attributes), so element accessors and non-default attributes use it too (H17 Element accessors row) |
| H18-Q3 | Where do non-array objects keep elements? | M1a | The +16 ELEMENTS slot of the H17 v0 layout. **Proposal 2026-10-27 (M4-P2 ADR-A0, Draft): as proposed, for every JS object kind.** The elements kind lives in the shape, so one shape compare guards it; an array-index key on a non-array goes to +16 and stops being an OM-10 trigger (H17 Elements for every kind row). ARGUMENTS already keeps its elements at +16; string wrappers and module namespaces keep their exotic hooks; typed arrays are their own kinds with off-heap data (H25) |
| H18-Q4 | When do the v0 limits go (2^26 cap, `length` past it)? | M1 | With dictionary elements. **Proposal 2026-10-27 (M4-P2 ADR-A0, Draft): with DICTIONARY elements in month 5 (ELEMENTS-KINDS).** 2^26 stays the dense limit, but an index or a `length` past it goes to DICTIONARY (Q2) instead of a string key or a RangeError, so `length` reaches 2^32 − 1 (ECMA-262) and the 59 `built-ins/Array` files that name 4294967294 or 4294967295 can pass |
| H18-Q5 | Frozen, sealed and non-extensible arrays | M1a | Separate kinds (06 §2.3). **Proposal 2026-10-27 (M4-P2 ADR-A0, Draft):** the kinds NONEXTENSIBLE, SEALED and FROZEN over ELEMENTS, each packed or holey, one-way in that order (V8 has the same six, external); a DOUBLE or INT32 array first moves to ELEMENTS. The FactSnapshot `ElementsKind` byte list (7 kinds, `crates/ep9_facts/src/bits.rs:178-193`) gets them, with the typed array, arguments and string wrapper kinds, in the one month-6 `FACT_SCHEMA_VERSION` bump (V9-D10) |

## Reopens if

- SP-Facts or Spike C cannot express an elements kind in FactSnapshot (H16) or in deopt metadata (H29).
- The M2 peak-RSS cap (G7) fails because of element storage.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft: plan row, v0 notes from the as-built arrays, open questions (M0 wk4 plan, task P0-DOCS 4; decision W4-H18) | P0-DOCS agent |
| 2026-10-21 | fill | v0 notes as built at the P3 merge: index keys move an object to dictionary mode (P2-OBJ, OM-10), the element-attribute limits (W4-ACC-ELEM), the prototype index-key flag (W4-P3FIX-ELEM-FLAG). Checked with `ep9` (debug, `479b1cd`) on a probe script | P4-DOCS agent |
| 2026-10-21 | clarify | 8 `path:line` references moved to the code at `479b1cd` (P4-DOCS drift script, [I15]); the RSS row names its commit; the index-key Rules row names its test | P4-DOCS agent |
| 2026-10-21 | clarify | 4 `path:line` references moved to the code after M2-P0 (drift script, [I15]) | M2-P0 integration fixer |
| 2026-10-22 | clarify | 3 `path:line` references moved to the code after the M2-P4 merge (drift script, [I15]). No check changes | P4 integration fixer |
| 2026-10-22 | clarify | 1 `path:line` reference moved to the code after the M2-P5 merge and the P5 fixer changes (drift script, [I15]). No check changes | P5 integration fixer |
| 2026-10-23 | clarify | 7 `path:line` references moved to the code after the M3-P0 merge and the M3-P0 code fixes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). The `delete a[i]` row cites the dense-array hole in `ordinary_delete` (set by a pin in the drift script; the script's content match had found another `if is_array` line). No check changes | M3-P0 integration fixer |
| 2026-10-23 | clarify | 3 `path:line` references in `crates/ep9_vm/src/property.rs` moved to the code after the M3-P0 round-2 code fix 02cd320 (X-CODE1: `get_data_from` grew 6 lines; drift script, [I15]). No check changes | M3-P0 round-2 docs fixer |
| 2026-10-23 | clarify | 3 `path:line` references moved to the code after the M3-P1 merge and the M3-P1 fixer changes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). No check changes | M3-P1 integration fixer |
| 2026-10-23 | clarify | 3 `path:line` references moved to the code after the M3-P2 merge and the M3-P2 fixer changes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS); each names the same item at the new line. No check changes | M3-P2 integration fixer (docs) |
| 2026-10-23 | clarify | 3 `path:line` references moved to the code after the M3-P2 fix round 2 merge (m0m3/p2-fix2-docs: the module, vm and perf fixer branches) (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS); each names the same item at the new line. The refs moved by the vm fixer's 1-line doc change in `object.rs` (`modf::ENVIRONMENT`, 25286b5). No check changes | M3-P2 fix round 2 (docs) |
| 2026-10-25 | clarify | 2 `path:line` references moved to the code (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS): `new_elements` and `array_set` in `crates/ep9_vm/src/object.rs` moved down 7 lines in 65d9b10 (C3-M1); each ref names the same item at the new line. No check changes | M3-P3 fix round 5 fixer |
| 2026-10-26 | clarify | 2 `path:line` references moved to the code after the M4-P1 merge (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS): `ep9_vm/src/object.rs` 744 -> 775 (`new_elements`) and 848 -> 879 (`array_set`); GC1-VERIFY f98bda3 (E2 check in `init`) and the M4-P1 fixer (633cb13, `alloc_from_shape`) added lines above them. Then the M4-P1 fixer's review fixes (84b1920, `OBJECT_FIELD0`) added 7 more: 775 -> 782 and 879 -> 886. Text only; no check changes | M4-P1 integration fixer |
| 2026-10-27 | fill (proposals; H18 is Draft) | H18-Q1 to H18-Q5 get proposals as the month-5 ELEMENTS-KINDS input (plan v9 §4.1 task 5; V9-D39): the hole NaN 0xFFF7_FFFF_FFF7_FFFF in a raw f64 cell kind, the sparse rule (gap 1,024, est.; the dense limit; one-way), elements for every JS object kind at +16, the v0 limits go with DICTIONARY elements, NONEXTENSIBLE, SEALED and FROZEN kinds (FactSnapshot list in the month-6 bump). Proposals only: the Decision, the v0 notes and every check keep their result | M4-P2 ADR-A0 (VM lead role) |
| 2026-10-27 | clarify (H18 is Draft) | Review fix from the M4-P2 Claude reviewer (ADR-A0 scope, F14): the H18-Q1 proposal said a DOUBLE store never writes the hole because boxing makes every NaN canonical; an unboxed f64 store from JIT or unit code can write 0xFFF7_FFFF_FFF7_FFFF (a `Float64Array` element read into an array). The proposal now says every DOUBLE-elements store makes NaN canonical, in every tier. No check changes | M4-P2 integration fixer B |
| 2026-10-27 | clarify | 3 `path:line` references moved to the code after the M4-P3 merges (GC3, KEYED, IC2, REGEXP1, TOOLS) and the M4-P3 integration fixer's code commits (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS; the ADRLINE run of fixer part B, after its code fixes): `crates/ep9_vm/src/property.rs` 321-325 -> 341-345, 312-318 -> 332-338, 597-604 -> 623-630. Each ref names the same line text as before (both ends of each range read at the ref's intro commit and at the new head). Text only; no check changes | M4-P3 integration fixer (part B) |
| 2026-10-27 | clarify (H18 is Draft) | 8 `path:line` references moved to the code after the M4-P4 merges (GC4, PROXY1, WEAKREF, MATH2, SPIKEC-4) and the M4-P4 integration fixer A's code commits (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS): `crates/ep9_vm/src/object.rs` 33 -> 34; `crates/ep9_vm/src/object.rs` 782 -> 810; `crates/ep9_vm/src/object.rs` 263-264 -> 264-265; `crates/ep9_vm/src/object.rs` 886 -> 914; `crates/ep9_vm/src/property.rs` 341-345 -> 362-366; `crates/ep9_vm/src/property.rs` 332-338 -> 353-359; `crates/ep9_vm/src/property.rs` 623-630 -> 693-700; `crates/ep9_facts/src/bits.rs` 172-187 -> 178-193. Each ref names the same line text as before (both ends of each range read at the ref's intro commit and at the new head). Text only; no check changes. Part B (M4-P4 integration fixer B, after its code commits edae2ba, cf6e643 and f29ca32; the same drift script): 3 more refs: `crates/ep9_vm/src/property.rs` 362-366 -> 370-374; `crates/ep9_vm/src/property.rs` 353-359 -> 361-367; `crates/ep9_vm/src/property.rs` 693-700 -> 743-750. Each names the same line text at dba9e30 and at the new head. Text only; no check changes. | M4-P4 integration fixer A; part B: M4-P4 integration fixer B |
| 2026-10-28 | clarify (H18 is Draft) | 3 `path:line` references moved to the code after the M5-P0 merges (P0-PLAT5, P0-VM5, GATES5, FUZZ-REC, CI5, DOC13) (`cargo xtask adr-lint --base m4-p5` FIX lines at 3c1c495; M2-P4-FIX-ADRLINE-ROWS): `crates/ep9_vm/src/object.rs` 810 -> 847; `crates/ep9_vm/src/object.rs` 264-265 -> 285-286; `crates/ep9_vm/src/object.rs` 914 -> 951. Each ref names the same line text as before (both ends of each range read at the ref's intro commit and at the M5-P0 fixer head). The Storage row ref to the ARRAY row of the `object.rs` module doc keeps line 21 with a pin (OBJECT_LAYOUT 5 added +32 to that line). Text only; no check, test or golden changes result (README §3.3 clarify). Review record (plan v10 rule 21a, §10): rows 1 and 3 the M5-P0 Claude reviewer (the fixer wrote this row), row 2 from the orchestrator's Codex texts, row 4 the fixer | M5-P0 integration fixer |
