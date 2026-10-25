# H19: Strings

| Field | Value |
|---|---|
| Status | Draft (2026-10-21; v0 notes). Flat Latin-1 and UTF-16 strings, atoms, keys and symbols are built; ropes, slices, thin and external strings are not |
| Fix by | M1a |
| Freeze point | M1a |
| Origin | v1#15; plan status U |
| Crates | `ep9_vm` (`string.rs`, `atoms.rs`, `key.rs`, `symbol.rs`, unit string loading in `unit.rs`); later `ep9_baseline`, `ep9_opt` (length and kind checks), `ep9_unit` (atoms by index, H13) |
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
| ATOM | word0 flag bit 0 (`FLAG_ATOM`) | `is_atom` (`crates/ep9_vm/src/string.rs:492`) |
| Hash field | 0 = not computed; bit 0 computed; bit 1 array index < 2^24 with the index in bits 2-25; else 30 hash bits in bits 2-31 (V8 Name style) | `crates/ep9_vm/src/string.rs:59` |
| Hash | SipHash-1-3 over UTF-16 code-unit values (LE), so both encodings hash alike; a cached index hashes its u32 value | `sip13_units_with` (`crates/ep9_vm/src/string.rs:678`) |
| Seed | one per process: random (std `RandomState`, OS source) or `EP9_HASH_SEED` (decimal, `0x` hex, or any text hashed to a seed) | `sip_keys` (`crates/ep9_vm/src/string.rs:586`) |
| Atom table | per isolate, `Rc<RefCell<AtomTable>>`; open addressing, linear probing, ≤ 1/2 full; weak: the heap weak source drops dead entries and rebuilds | `crates/ep9_vm/src/atoms.rs:113`; `init` (`crates/ep9_vm/src/atoms.rs:264`); `crates/ep9_heap/src/heap.rs:950` |
| Interning | a string cell becomes the atom in place (no allocation); text from Rust: hash, probe, allocate first, then insert | `intern` (`crates/ep9_vm/src/atoms.rs:280`), `intern_text` (`crates/ep9_vm/src/atoms.rs:297`) |
| `atoms!` list | 35 names, `pub enum Atom` `repr(u16)`, append-only ids; isolate roots | `crates/ep9_vm/src/atoms.rs:60` |
| Single characters | 256 Latin-1 atoms, isolate roots, filled lazily; `s[i]` and the string iterator use them | `char_at` (`crates/ep9_vm/src/string.rs:371`) |
| Units | every unit string is interned at load (root range of the unit) | `crates/ep9_vm/src/unit.rs:148` |
| Keys | ToPropertyKey flattens and interns; no number keys: "1" is an atom with the index cached (W4-NO-NUMBER-KEYS) | `crates/ep9_vm/src/key.rs:56` |
| Key compare | same bits, or two atoms → unequal at once; a non-atom key takes one content compare and bumps a debug counter | `key_eq` (`crates/ep9_vm/src/key.rs:43`), `probe` (`crates/ep9_vm/src/key.rs:153`) |
| Limit | 2^29 − 24 code units (V8); `concat` returns `None` and callers throw RangeError "Invalid string length" | `crates/ep9_vm/src/string.rs:54`; `crates/ep9_vm/src/natives.rs:1374` |
| Read API | `flatten(cx, s)` (identity in v0), then `view(&cx, s) -> JsStr<'cx>`; legacy `str_view` sites move by M1a | `crates/ep9_vm/src/string.rs:276` |

- **Why a `JsStr` is safe in VM code.** The heap never moves cells (S0-S2), and the conservative scan keeps a cell alive from an interior pointer (`find_cell`, `crates/ep9_heap/src/trace.rs:357`). So a view held in a Rust local stays valid across an allocation. The borrow rule (`view` borrows `&Cx`) protects builtins, which get no `&Runtime` (H20 `abi::Cx`), and a future moving GC (S3) [I49].
- **The embedder view.** `Isolate::string_view` does not root the string, and the embedder can keep a view where the conservative scan does not look (a `Vec`). So it borrows the isolate with `&mut self`: no isolate call runs while the view lives. A shared borrow is not enough, because a `&self` call can allocate or collect: in the in-crate tests and the spike-native build, `Isolate::heap` (`Heap::collect`, `Heap::alloc`, `Heap::alloc_shape`). The raw `Isolate::global` (it interned its name) was one more until M3-P3 removed it, and the default build has no `Isolate::heap` since M3-P3 fix round 5 (H20-Q4). A raw `Value` is not a root, so `string_view` also checks it before it reads it: it reads only a live string cell of this isolate's heap (`Heap::is_cell`), and gives `None` for a freed cell, a cell of another heap or bits that do not start a cell (H20-Q4, Codex W4-1).
- **GC rule** (W4-GC-RULES). The weak source holds an `Rc` clone of the table and borrows it during a GC. Interning never holds a borrow across an allocation: it allocates first, then borrows and inserts. A held borrow makes the GC panic ("borrowed across a GC point").

**Symbols (v0, as built, M0 week 4).** A property key is a string atom or a symbol. Both compare by bits: an atom is the one cell of its text, a symbol is its own identity.

| Item | v0 behavior | Where |
|---|---|---|
| Cell | SYMBOL (kind 8), 32 B: +8 `[[Description]]` (a string or undefined), +16 identity hash (int32, 30 bits), +24 flags (int32: bit 0 registered, bit 1 well-known, bit 2 private, reserved for M1) | `crates/ep9_vm/src/symbol.rs:59` |
| Identity hash | the next value of a per-isolate splitmix64 PRNG (H12-Q3), never the address. Seed: 60 bits of the per-process SipHash key, read through the keyed hash of two fixed texts, so `EP9_HASH_SEED` fixes it and every isolate of one process starts the same sequence. 30 bits, the range of a string's table hash, so one shape index serves both | `SymbolState` (`crates/ep9_vm/src/symbol.rs:249`), `prng_seed` (`crates/ep9_vm/src/symbol.rs:292`) |
| Well-known symbols | the 13 of §6.1.5.1 plus `dispose` and `asyncDispose`; descriptions "Symbol.iterator" and so on; made by `symbol::install` in realm init; isolate roots in `WellKnownSymbol` order | `install` (`crates/ep9_vm/src/symbol.rs:387-409`), `well_known` (`crates/ep9_vm/src/symbol.rs:412`) |
| `%Symbol.prototype%` | an ordinary object whose prototype is `%Object.prototype%`; the prototype of Symbol wrappers; a symbol root until the builtin tables give it an intrinsic slot (P2-BTAB) | `prototype` (`crates/ep9_vm/src/symbol.rs:418`) |
| `Symbol.for` registry | per isolate; open addressing keyed by the key's content hash (never an address); keys are atoms, so a probe compares bits; strong: a registered symbol lives as long as the isolate. `key_for` is the description of a registered symbol | `symbol_for` (`crates/ep9_vm/src/symbol.rs:461`), `key_for` (`crates/ep9_vm/src/symbol.rs:479`) |
| Roots | one precise root source (it holds an `Rc` clone): the well-known symbols, `%Symbol.prototype%`, the class-fields key (H19-Q7), the identity-hash key (`symbol::identity_hash_key`, H12-Q1; M3-P1), then every registry entry. `symbol_for` allocates first, then borrows the registry and inserts (W4-GC-RULES) | `install` (`crates/ep9_vm/src/symbol.rs:387-409`) |
| Keys | ToPropertyKey gives a symbol back as it is (also when ToPrimitive returns one); `probe` classifies a symbol as Bits; `key_hash` gives its identity hash; `key_text` prints `Symbol(desc)` | `crates/ep9_vm/src/key.rs:56`, `:153`, `:166`, `:130` |
| Conversions | `typeof` "symbol"; ToBoolean true; ToNumber and ToString throw TypeError ("Cannot convert a Symbol value to a number" / "to a string", as V8); ToPrimitive calls `@@toPrimitive` with "default", "number" or "string"; IsLooselyEqual steps 11-12 take symbols | `crates/ep9_vm/src/conv.rs:82`, `:143`, `:189`, `:224`, `:275` |
| Text | SymbolDescriptiveString as Rust text for messages and `describe` (`descriptive_text`), and as a heap string (`descriptive_string`, RangeError past the length limit) | `crates/ep9_vm/src/symbol.rs:497`, `:508` |
| Wrappers | PRIMITIVE_WRAPPER with one fixed field through the fixed-field API; ToObject wraps a string, number, boolean or symbol with the realm's prototype; a String wrapper has own `length` {w: f, e: f, c: f} and indices {w: f, e: t, c: f} | `new_wrapper` (`crates/ep9_vm/src/wrapper.rs:29`), `to_object` (`crates/ep9_vm/src/conv.rs:240`), `string_own_property` (`crates/ep9_vm/src/wrapper.rs:66`) |

- **Why a PRNG and not a counter.** A counter makes hashes of symbols made in a row adjacent, which clusters them in a power-of-two hash index. H12-Q3 proposes a per-isolate PRNG; V8 also takes symbol hashes from the isolate's random number generator. The seed follows the string hash seed, so a run with `EP9_HASH_SEED` is reproducible.
- **Why the registry is strong.** `Symbol.for(k)` must return the same symbol for the life of the realm, and a program can observe identity (`===`) even when it holds no reference in between. A weak registry would need the "symbol can be held weakly" rules of §9.13 CanBeHeldWeakly; registered symbols cannot be.

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
| A symbol is its own key: compared by bits, hashed by its identity hash, listed after the strings by [[OwnPropertyKeys]] | `a_symbol_works_as_a_key`, `own_property_keys_put_symbols_last_in_creation_order`, `symbol_keys_in_shapes_compare_by_bits_and_hash_by_identity` (`crates/ep9_vm/src/tests/symbols.rs`) | M0 wk4 |
| Identity hashes come from the isolate PRNG, never the address; one seed gives one sequence | `two_isolates_with_one_seed_give_the_same_symbol_hashes` (two isolates, and two child processes with `EP9_HASH_SEED`) | M0 wk4 |
| Well-known and registered symbols survive every GC; `Symbol.for` gives one symbol per key text | `well_known_symbols_are_isolate_roots_in_order`, `the_registry_is_stable` (GC stress, verifier, liveness check) | M0 wk4 |
| `${s}` and `+s` throw TypeError; `@@toPrimitive` gets the spec hint | `template_literals_and_unary_plus_throw_on_a_symbol`, `at_at_to_primitive_gets_the_right_hint` | M0 wk4 |
| ToObject wraps each primitive; String wrapper `length` and indices have the spec attributes | `crates/ep9_vm/src/tests/wrappers.rs` | M0 wk4 |
| A view from `view(&cx, ..)` cannot live across a `&mut` call; a view from `Isolate::string_view` cannot live across any isolate call | compile_fail doctests on `Isolate::string_view`: `collect_garbage` (E0499), views in a `Vec` across the `&self` call `gc_stats()` (E0502); rustdoc checks the codes on nightly | M0 wk4 |
| No new `str_view` call site; 0 sites by the freeze | `grep -rn str_view crates/ep9_vm/src` (W4-STRVIEW) | M1a |

## Consequences

- **Gains.** A key compare is one word for atoms, the same in T0, ICs, JIT and AOT. Strings save 8 B against the week-3 header. `s.length` is one load for every kind, ropes included. Index keys stay strings, so property.rs needs no number-key path this week.
- **Costs.** Every unit string and computed key pays one hash and one table probe. Every GC walks the atom table (O(capacity)). A text over the limit built from Rust (for example an error message that quotes a huge key) still panics in `alloc_string` (H19-Q5).
- **Follow-ups.** Ropes and slices (M1); P2-OBJ calls `intern_key` on define paths so stored keys are always atoms; symbol keys in `probe` and `key_hash` (done, see "Symbols"); `===` of two atoms can skip the content compare (`conv.rs`, `interp/ops.rs`).

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
| H19-Q7 | Private names (`#x`): a SYMBOL with the private flag (V8) or a kind of their own | M1 (classes) | **Filled 2026-10-22 (as built, M0 month 2 P2): a SYMBOL with the private flag, bit 2** (V8). `CreatePrivateName(description)` (runtime id 23) makes a new one per evaluation of a class (`symbol::new_private`, `crates/ep9_vm/src/symbol.rs:336`). A private element is an own entry of the object's storage keyed by that symbol (attributes WRITABLE); a class with private methods or accessors adds one private-name brand per instance (or to the constructor for static ones). `[[OwnPropertyKeys]]` and for-in never list a private key (`crates/ep9_vm/src/property.rs:1262`); the keyed opcodes send it to `interp/slow/private.rs` (own lookup only, no IC, C7). An add on a non-extensible object throws a TypeError (M2-PRIVATE-EXT). One more private symbol per isolate, the class-fields key (`class_fields_key`, `crates/ep9_vm/src/symbol.rs:445`), holds a class's instance-field initializer on its constructor (C8). Since M3-P1 a second one, the identity-hash key (`identity_hash_key`, made by `symbol::install` after the class-fields key), holds an object's identity hash (H12-Q1); both are one per isolate, shared by every realm |

## Reopens if

- The M1a string speed check (02 §6 H19) misses the V8/SM class on the string workloads.
- The atom-table check fails under GC stress, or the content-compare counter cannot reach 0 once P2-OBJ interns stored keys.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft: plan row, v0 notes as built (header, hash field, SipHash-1-3 with a per-process seed, weak atom table, `atoms!` ids, single-character atoms, unit interning, keys without number keys, limit, read API), rules, open questions (M0 wk4 plan, task P1-STR 11; decisions W4-STRINGS, W4-HASH, W4-KEY-EQ, W4-NO-NUMBER-KEYS) | P1-STR agent |
| 2026-10-21 | fix | `Isolate::string_view` takes `&mut self`: with `&self`, views kept in a `Vec` read freed memory after `heap().collect()` or a `global()` call that collected (P1 review finding). Two more compile_fail doctests; the embedder-view note | P1 fix agent |
| 2026-10-21 | fill | Symbols v0: the SYMBOL cell, the identity hash from a per-isolate PRNG seeded by the string hash key (H12-Q3), the well-known symbols as roots, `%Symbol.prototype%`, the strong `Symbol.for` registry, symbol keys, the conversions and the primitive wrappers; five rules with their tests; H19-Q7 (private names) | P2-SYM agent |
| 2026-10-21 | clarify | 20 `path:line` references moved to the code at `479b1cd` (P4-DOCS drift script, [I15]) | P4-DOCS agent |
| 2026-10-21 | clarify | 18 `path:line` references moved to the code after M2-P0 (drift script, [I15]) | M2-P0 integration fixer |
| 2026-10-22 | fill | H19-Q7 filled (as built): private names are SYMBOLs with the private flag; private elements are own entries keyed by them (own lookup only, never listed); the class-fields key is a new symbol root. Decisions M2-PRIVATE-EXT, C7, C8 (decisions log §12). Also 11 `path:line` references into symbol.rs moved to the new lines (drift script, [I15]) | P2-CLS-VM agent |
| 2026-10-22 | clarify | 2 `path:line` references moved to the code after the M2-P4 merge (drift script, [I15]). No check changes | P4 integration fixer |
| 2026-10-22 | clarify | 2 `path:line` references moved to the code after the M2-P5 merge and the P5 fixer changes (drift script, [I15]). No check changes | P5 integration fixer |
| 2026-10-23 | clarify | 12 `path:line` references moved to the code after the M3-P0 merge and the M3-P0 code fixes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). No check changes | M3-P0 integration fixer |
| 2026-10-23 | clarify | 1 `path:line` reference in `crates/ep9_vm/src/property.rs` moved to the code after the M3-P0 round-2 code fix 02cd320 (X-CODE1: `get_data_from` grew 6 lines; drift script, [I15]). No check changes | M3-P0 round-2 docs fixer |
| 2026-10-23 | clarify | The Roots row and H19-Q7 name the second per-isolate private symbol, the identity-hash key (`symbol::identity_hash_key`, made by `symbol::install` after the class-fields key; H12-Q1), as M3-P1 built it (fix-realm, review finding COLL-1). No rule or check changes | M3-P1 integration fixer |
| 2026-10-23 | clarify | 14 `path:line` references moved to the code after the M3-P1 merge and the M3-P1 fixer changes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). No check changes | M3-P1 integration fixer |
| 2026-10-23 | clarify | 1 `path:line` reference moved to the code after the M3-P2 merge and the M3-P2 fixer changes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS); each names the same item at the new line. No check changes | M3-P2 integration fixer (docs) |
| 2026-10-25 | clarify | The embedder-view paragraph no longer names `Isolate::global` as a live `&self` call that allocates: M3-P3 EMBED-MIGRATE removed the raw `Isolate::{run_script, call, global, display}` (H20-Q4). `Isolate::heap` stays the example. No rule or check changes | M3-P3 EMBED-MIGRATE (step 2) |
| 2026-10-25 | clarify | 1 `path:line` reference moved to the code after the M3-P3 fix round 2 change (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS): `LoadedUnit` got the `script_or_module` field (the ScriptOrModule of eval code for `import()`). The ref names the same item at the new line. No check changes | M3-P3 fix round 2 (import() and TLA) |
| 2026-10-25 | rewrite | The embedder view after M3-P3 fix round 5 (H20-Q4; Codex W4-1). `Isolate::heap` is gone from the default build, so 2 `compile_fail` doctests on `Isolate::string_view` change: the one across `heap().collect()` now crosses `gc_stats()` (still E0502), and the one across `heap().alloc(..)` goes (that call does not compile there for another reason, E0599, and the `NoRawHeap` doctests of `isolate.rs` hold that check). The check row also no longer names a `global` doctest, which M3-P3 EMBED-MIGRATE had replaced. `string_view` now reads only a live string cell of its own heap; the embedder-view paragraph says so. The kind is rewrite because a check changes; H19 is before its freeze point (M1a) | M3-P3 fix round 5 fixer |
| 2026-10-25 | clarify | 4 `path:line` references moved to the code (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS): `is_atom`, `sip13_units_with`, `sip_keys` and `char_at` in `crates/ep9_vm/src/string.rs` moved down 22 lines in M3-P3 fix round 5 (1d3ce33: the `string_view` doc text and its check); each ref names the same item at the new line. No check changes | M3-P3 fix round 5 fixer |
