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
| Storage | ARRAY cell plus one ELEMENTS cell of Values; a hole is the H6 empty value 0x22 | `crates/ep9_vm/src/object.rs:21`, `crates/ep9_vm/src/object.rs:31`; `new_elements` (`crates/ep9_vm/src/object.rs:589`) |
| Dense limit | 2^26 elements (`arr::MAX_LENGTH`) | `crates/ep9_vm/src/object.rs:160-161` |
| Growth | a store at index i makes capacity max(i + 1, 2 × capacity, 4) | `array_set` (`crates/ep9_vm/src/object.rs:626`) |
| High dense index | `a[2**26 - 1] = 1` allocates 2^26 slots: 540,606,464 B peak RSS (empty script: 3,473,408 B) | measured 2026-10-21 at `dabc38a`, release `ep9`, M5 Max (not measured again; the growth rule is unchanged) |
| Index at or above 2^26 | stored as a string-keyed property, so the array moves to dictionary mode (an index key never enters a shape, OM-10); `length` does not change (ECMA-262: index + 1) | `crates/ep9_vm/src/property.rs:274-278` falls through |
| `length` above 2^26 | RangeError "Invalid array length" (ECMA-262 allows up to 2^32 - 1) | `crates/ep9_vm/src/property.rs:265-271` |
| `delete a[i]` | writes a hole | `crates/ep9_vm/src/property.rs:511-518` |
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
| H18-Q1 | The hole NaN bit pattern for DOUBLE, next to the canonical NaN (H06-Q3) | M1a | A NaN payload that purification never makes |
| H18-Q2 | The sparse rule: when does a store go to dictionary elements? | M1a | A gap rule on (index − length) and a fill ratio, as V8 |
| H18-Q3 | Where do non-array objects keep elements? | M1a | The +16 ELEMENTS slot of the H17 v0 layout |
| H18-Q4 | When do the v0 limits go (2^26 cap, `length` past it)? | M1 | With dictionary elements |
| H18-Q5 | Frozen, sealed and non-extensible arrays | M1a | Separate kinds (06 §2.3) |

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
