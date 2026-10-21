# H19: Strings

| Field | Value |
|---|---|
| Status | Draft (2026-10-21; v0 notes). Flat Latin-1 and UTF-16 strings, atoms and keys are built; ropes, slices, thin and external strings are not |
| Fix by | M1a |
| Freeze point | M1a |
| Origin | v1#15; plan status U |
| Crates | `ep9_vm` (`string.rs`, `atoms.rs`, `key.rs`, unit string loading in `unit.rs`); later `ep9_baseline`, `ep9_opt` (length and kind checks), `ep9_unit` (atoms by index, H13) |
| Owner (plan role) | VM lead |
| Validated by | `crates/ep9_vm/src/tests/strings.rs`; the debug atom-table check after every GC; the `EP9_ASSERT_ATOM_KEYS` gate; test262 with and without `--gc-stress` |
| Plan sources | [02 §6 H19](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [06 §3](../plan/06-object-model-and-spec.md#3-strings-h19-unchanged--atom-indices) |

## Context

- Goal: string speed of the V8 and SpiderMonkey class [R06]. Property keys, shapes and ICs compare one word, so a key must be one cell per text.
- G6: hostile input must not degrade the engine (hash flooding, huge strings).
- Depends on: H6 (cell values), H8 (word0 kind byte and flags), H10 (CSS keeps interior pointers), H12 (no address hashing), H13 (stable atom indices). Depended on by: H17 (shape keys), H20 (the builtin read API), H35 (units name atoms by index).

## Decision

SeqLatin1, SeqUtf16, rope (length ≥ 13), slice (length ≥ 13), thin, external. Atoms with a cached hash and an array-index bit. Never a Rust `String` as the JS representation. Snapshot atoms use H13 indices (02 §6).

**v0 notes (as built, M0 week 4).**

| Item | v0 behavior | Where |
|---|---|---|
| Kinds | SEQ_LATIN1 (1) and SEQ_UTF16 (2) built; 3-7 (cons, sliced, thin, external ×2) reserved, same header | `crates/ep9_vm/src/kinds.rs`; `register_kinds` (`crates/ep9_vm/src/string.rs:100`) |
| Header | u32 length at +8, u32 hash field at +12, code units at +16; size 16 + len or 16 + 2·len; trace none | `crates/ep9_vm/src/string.rs:46` |
| ATOM | word0 flag bit 0 (`FLAG_ATOM`) | `is_atom` (`crates/ep9_vm/src/string.rs:450`) |
| Hash field | 0 = not computed; bit 0 computed; bit 1 array index < 2^24 with the index in bits 2-25; else 30 hash bits in bits 2-31 (V8 Name style) | `crates/ep9_vm/src/string.rs:59` |
| Hash | SipHash-1-3 over UTF-16 code-unit values (LE), so both encodings hash alike; a cached index hashes its u32 value | `sip13_units_with` (`crates/ep9_vm/src/string.rs:636`) |
| Seed | one per process: random (std `RandomState`, OS source) or `EP9_HASH_SEED` (decimal, `0x` hex, or any text hashed to a seed) | `sip_keys` (`crates/ep9_vm/src/string.rs:544`) |
| Atom table | per isolate, `Rc<RefCell<AtomTable>>`; open addressing, linear probing, ≤ 1/2 full; weak: the heap weak source drops dead entries and rebuilds | `crates/ep9_vm/src/atoms.rs:113`; `init` (`crates/ep9_vm/src/atoms.rs:264`); `crates/ep9_heap/src/heap.rs:696` |
| Interning | a string cell becomes the atom in place (no allocation); text from Rust: hash, probe, allocate first, then insert | `intern` (`crates/ep9_vm/src/atoms.rs:280`), `intern_text` (`crates/ep9_vm/src/atoms.rs:297`) |
| `atoms!` list | 35 names, `pub enum Atom` `repr(u16)`, append-only ids; isolate roots | `crates/ep9_vm/src/atoms.rs:60` |
| Single characters | 256 Latin-1 atoms, isolate roots, filled lazily; `s[i]` and the string iterator use them | `char_at` (`crates/ep9_vm/src/string.rs:329`) |
| Units | every unit string is interned at load (root range of the unit) | `crates/ep9_vm/src/unit.rs:82` |
| Keys | ToPropertyKey flattens and interns; no number keys: "1" is an atom with the index cached (W4-NO-NUMBER-KEYS) | `crates/ep9_vm/src/key.rs:53` |
| Key compare | same bits, or two atoms → unequal at once; a non-atom key takes one content compare and bumps a debug counter | `key_eq` (`crates/ep9_vm/src/key.rs:40`), `probe` (`crates/ep9_vm/src/key.rs:115`) |
| Limit | 2^29 − 24 code units (V8); `concat` returns `None` and callers throw RangeError "Invalid string length" | `crates/ep9_vm/src/string.rs:54`; `crates/ep9_vm/src/natives.rs:306` |
| Read API | `flatten(cx, s)` (identity in v0), then `view(&cx, s) -> JsStr<'cx>`; legacy `str_view` sites move by M1a | `crates/ep9_vm/src/string.rs:256` |

- **Why a `JsStr` is safe in VM code.** The heap never moves cells (S0-S2), and the conservative scan keeps a cell alive from an interior pointer (`find_cell`, `crates/ep9_heap/src/trace.rs:310`). So a view held in a Rust local stays valid across an allocation. The borrow rule (`view` borrows `&Cx`) protects builtins, which get no `&Runtime` (H20 `abi::Cx`), and a future moving GC (S3) [I49].
- **The embedder view.** `Isolate::string_view` does not root the string, and the embedder can keep a view where the conservative scan does not look (a `Vec`). So it borrows the isolate with `&mut self`: no isolate call runs while the view lives. A shared borrow is not enough, because some `&self` calls allocate or collect: `Isolate::global` (it interns its name) and `Isolate::heap` (`Heap::collect`, `Heap::alloc`, `Heap::alloc_shape`).
- **GC rule** (W4-GC-RULES). The weak source holds an `Rc` clone of the table and borrows it during a GC. Interning never holds a borrow across an allocation: it allocates first, then borrows and inserts. A held borrow makes the GC panic ("borrowed across a GC point").

## Rules

| Rule | Check | From |
|---|---|---|
| `""` is 16 B, `"abc"` 24 B; length is the u32 at +8 for every string kind | `empty_string_is_16_bytes_and_abc_is_24` | M0 wk4 |
| One atom per text per isolate; each atom has ATOM and sits under its own hash | `AtomTable::check` after every GC and `atoms::verify` at isolate drop (debug builds); `atoms_verify_runs_and_finds_a_broken_table` | M0 wk4 |
| Unit strings and ToPropertyKey results are atoms; "1" has the index bit and index 1 | `one_name_from_two_units_and_a_concat_key_are_one_cell`, `index_strings_are_atoms_with_the_index_cached` | M0 wk4 |
| The table is weak: an unreferenced atom goes at the next GC, a referenced one stays | `unreferenced_dynamic_atoms_go_and_referenced_ones_stay` | M0 wk4 |
| Latin-1 and UTF-16 forms of one text hash alike | `latin1_and_utf16_forms_hash_alike` (10^4 texts) | M0 wk4 |
| The seed is per process; `EP9_HASH_SEED` fixes it; no hash in bytecode, units, snapshots or property order | `ep9_hash_seed_gives_the_same_hashes_in_two_runs`; review rule (H12) | M0 wk4 |
| Key compares on atoms are bit compares; the content-compare counter stays 0 | `EP9_ASSERT_ATOM_KEYS=1 cargo test -p ep9_vm` (debug); `the_atom_key_gate_fails_on_a_non_atom_compare` | M0 wk4 |
| An over-limit result throws RangeError, never panics | `over_limit_strings_throw_range_error` | M0 wk4 |
| A view from `view(&cx, ..)` cannot live across a `&mut` call; a view from `Isolate::string_view` cannot live across any isolate call | compile_fail doctests on `Isolate::string_view`: `collect_garbage` (E0499), views in a `Vec` across `heap().collect()` (E0502), `global` (E0502); rustdoc checks the codes on nightly | M0 wk4 |
| No new `str_view` call site; 0 sites by the freeze | `grep -rn str_view crates/ep9_vm/src` (W4-STRVIEW) | M1a |

## Consequences

- **Gains.** A key compare is one word for atoms, the same in T0, ICs, JIT and AOT. Strings save 8 B against the week-3 header. `s.length` is one load for every kind, ropes included. Index keys stay strings, so property.rs needs no number-key path this week.
- **Costs.** Every unit string and computed key pays one hash and one table probe. Every GC walks the atom table (O(capacity)). A text over the limit built from Rust (for example an error message that quotes a huge key) still panics in `alloc_string` (H19-Q5).
- **Follow-ups.** Ropes and slices (M1); P2-OBJ calls `intern_key` on define paths so stored keys are always atoms; P2-SYM adds symbol keys to `probe` and `key_hash`; `===` of two atoms can skip the content compare (`conv.rs`, `interp/ops.rs`).

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R06] | V8 kinds: SeqOneByte, SeqTwoByte, Cons (min 13), Thin, Sliced (min 13), External; internalized atoms with a cached hash | [06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R08] | Build our own Latin-1/UTF-16 strings with ropes, slices, thin strings and atoms; lone surrogates force UTF-16 | [08-spec-subsystems.md](../research/08-spec-subsystems.md) |
| [CVE] | Node CVE-2017-11499: a fixed hash seed in the V8 snapshot allowed hash flooding | decisions-log W4-HASH |
| [M] | Isolate creation and test262 counts in the change log below | M0 wk4 P1-STR report |

## Validation

- Unit tests in `crates/ep9_vm/src/tests/strings.rs`, also under `gc_stress: 1` with the heap verifier.
- test262 exec: 0 new failures, with and without `--gc-stress 1`.
- M1a: rope and slice tests; flatten under GC stress; the `str_view` count at 0.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H19-Q1 | When does a rope flatten (index, compare, atomize)? | M1 | V8: flatten on index and on atomize |
| H19-Q2 | Atom lookup without insert for [[Get]] | M1 | Probe only; a miss means "no such key" |
| H19-Q3 | How units and snapshots name atoms by index (H13) | M1 (lists), A1 | `Atom` ids first, then a unit atom table |
| H19-Q4 | Thin strings: when can a string not become an atom in place? | M1 | Ropes and external strings get a thin forwarder |
| H19-Q5 | Over-limit text built from Rust (messages) | M1 | Truncate quoted values in messages; RangeError elsewhere |
| H19-Q6 | A number-to-string cache for `o[i]` on non-arrays | M1 | V8-style small cache keyed by the number |

## Reopens if

- The M1a string speed check (02 §6 H19) misses the V8/SM class on the string workloads.
- The atom-table check fails under GC stress, or the content-compare counter cannot reach 0 once P2-OBJ interns stored keys.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft: plan row, v0 notes as built (header, hash field, SipHash-1-3 with a per-process seed, weak atom table, `atoms!` ids, single-character atoms, unit interning, keys without number keys, limit, read API), rules, open questions (M0 wk4 plan, task P1-STR 11; decisions W4-STRINGS, W4-HASH, W4-KEY-EQ, W4-NO-NUMBER-KEYS) | P1-STR agent |
| 2026-10-21 | fix | `Isolate::string_view` takes `&mut self`: with `&self`, views kept in a `Vec` read freed memory after `heap().collect()` or a `global()` call that collected (P1 review finding). Two more compile_fail doctests; the embedder-view note | P1 fix agent |
