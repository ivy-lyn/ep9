# Best-fit data structures for heap containers, and the oxc optimization study

## Run context

| Item | Value |
|---|---|
| Date | 2026-11-18 |
| User order | "start another round of performance optimization: use best fit data structs for heap allocated stuffs. For example, use a faster HashMap impl if the use case is not security sensitive. Deepresearch oxc and study their optimization" |
| ep9 tree | tag `m9-p5`, 86e39c65 (`86e39c6530371bac073162c70b9461f06328831c`). Every ep9 line cite in this report is at this commit |
| oxc | `oxc-project/oxc@3464a0d18` (`3464a0d181f0eb8e05b9c35d954e9a7d43d6124d`, 2026-11-16, local clone); crates.io `oxc_str` and `oxc_semantic` 0.158.0, the versions in ep9's Cargo.lock |
| Profile | The M9-P5 Bim cachegrind files (x64, jit and jitless, head 56dd9df4). `git diff --stat 56dd9df4 86e39c65` is empty on atoms.rs, string.rs, shape.rs, abi/mod.rs, array.rs, props.rs and generator.rs; key.rs changed one line (the PREALLOC K1 `intern_number` line). So the profile lines still match the tree. The M9-P4 bench logs give the allocator and insert rows |
| Method | 5 gatherers (ep9 inventory, ep9 profile, oxc history, oxc code, security classes), then a synthesis, then 2 adversarial verifiers (security, gain). The verdicts are applied in place: refuted items are in §6, corrected numbers replace the old ones |
| Last round | PREALLOC (pre-size known containers): K1 splay -1.2%, K2-K18 TS -0.22..-0.24% Ir (orchestrator log M6-ORCH-1012). This report does not repeat its items (§6.3) |
| Kill line | A lever must move >= 0.2% Ir on one bench row. The bench is cachegrind Ir on 19 rows (t0-*, 7 v8-v7 kernels, Octane-7) plus the TS set, x64 and a64, jit and jitless builds |

Short forms:
- `Hnn` = `docs/adr/Hnn-*.md`, DL = [docs/decisions-log.md](../decisions-log.md). `H19:35` means line 35 of H19 at `m9-p5`. To read a cite later, use `git show m9-p5:<path>`.
- oxc cites read `oxc-project/oxc@3464a0d18 <path>:<line>`, or a commit id with its PR number. PR links are `https://github.com/oxc-project/oxc/pull/<n>`.
- `CG <file>:<line>` = the M9-P5 Bim cachegrind output `<file>` (x64 jit, head 56dd9df4); `CG-jitless` is the jitless copy. They were read with the session parsers `scratchpad/bf/cg.py`, `scratchpad/bf/sip.py` and `scratchpad/vf/fn.py` (session-local).
- `bench log <run id> <job id>:<line>` = an orchestrator bench log.
- "est." marks an estimate. A number without "est." was read in a profile or a source.

---

## 0. Verdict

```
est. removable share of the row, x64 (1 char = 0.1% Ir)   kill line 0.2% = ██
B1 static atom text -> its root (deltablue)   |██████████████▌ 1.45%  -> est. -1.3..-1.4%   KEEP
B2 intern guard split (t0-gen)                |██████████████████████████▍ 2.64% -> est. -2.6..-2.8%  KEEP, call shape (hand back)
B3+B5 std maps on the compile path (e-b)      |▌ 0.05%                                    WEAK
ALL std SipHash + hashbrown (earley-boyer)    |█▋ 0.174% (jitless 0.183%)  below the kill line even if all of it went away
```

- **The "faster HashMap" ask, as asked, cannot pass the kill line.** All std SipHash cost (sip, std hash, hashbrown, oxc's own maps included) is compile time or start-up. Its peak is 0.174% of earley-boyer x64 jit (0.183% jitless). The engine-id maps (class (b), §3) hold about 0.05% of that.
- **The real hash cost is in ep9's own atom path, at call sites that re-intern static Rust text.** `Array.prototype.pop` interns `"length"` twice per call. Each time it runs a seeded SipHash of the text, a table probe and a compare, but the answer is already a root: `Atom::Length`.
- **B1 is the lever.** A fixed-set `match` from text to atom gives the same bits for est. 10-20 Ir. It is the oxc lesson "a small fixed set needs no general hash table", applied to a runtime.
- **B2 is real but is a call shape, not a container.** It goes back to the orchestrator as its own micro-item.
- **Security:** no unkeyed fast hash goes on a key that JS or source text can choose. §7 has the rule. The study found 5 hash weaknesses that are already in the tree; they are hand-back items (§8), not levers.

---

## 1. What oxc does, and what fits a runtime

| # | oxc lesson | oxc source | Measured gain (oxc's own numbers) | Fits ep9? | ep9 item |
|---|---|---|---|---|---|
| O1 | A small fixed string set needs no general hash map. oxc matches keywords with a SIMD perfect hash and a slot table (not a `match`) | `oxc-project/oxc@3464a0d18 crates/oxc_lexer/src/pipeline/keywords.rs:405-438`; a3571bdd7 (#29036); oxc perf page: the "simple keyword match is enough" note | a3571bdd7: "avx2 main set about 1% faster" (one keyword hash against another). The perf page: keyword lookup was "only about 1 - 2% of the performance" (https://oxc.rs/docs/learn/performance.html) | **Yes.** The 36 static atoms are a fixed set | B1 |
| O2 | A small inline guard, with the big body `#[cold] #[inline(never)]` | ab22e80f4 (#24354, a Rust 1.97 inlining regression in the transformer walker); b4953b4e3 (#10675); a6c11fa95 (#23157) | b4953b4e3: "a 3%-4% performance improvement in the minfier" (a reserve call inside `Vec::extend`; it shows the pattern works, the number does not carry over) | **Yes**, for call shape. Each use needs a FRAME-GOLDEN check (rule 81) | B2 |
| O3 | A dense id key goes in an `IndexVec` or `Vec<Option<_>>`, not a hash map | 217d7d8a1 (#22441); e5baf60fb (#19221); ba35a0d2e (#24549) has the criteria: a dense id, a sound bound at construction, allocated once per pass, not per inner loop | #22441: median -207 µs (-2.97%) minifier, 27 of 27 cycles. #19221: vue-id.ts 8.53 ms -> 6.57 ms | **Yes**, for compile-side maps keyed by engine ids. The share is tiny on ep9 rows | B4 |
| O4 | One flat `(scope, name)` map, or a pool of cleared per-scope maps | d5c916ad9 (#23927); 9f6ee3b41 (#23761) | #23927: antd.js semantic system allocs 1020 -> 28. #23761: isolated-declarations about 13.6% faster (5.24 -> 4.53 ms) | **Only with a seeded hasher.** A flat Fx map on `(ScopeId, NameId)` is attackable (§6.1, B5) | B5 (safe form) |
| O5 | Fx for internal keys, with a clippy `disallowed-types` ban on std `HashMap`/`HashSet` | `oxc-project/oxc@3464a0d18 .clippy.toml:17-20`; `Cargo.toml:202` (`rustc-hash = "2"`) | none given | **Only for class (b) keys.** A ban copied as-is would push Fx onto JS text and numbers (§7) | B3 |
| O6 | Hash an identifier once at lex time and keep the hash in the `Ident` | `oxc-project/oxc@3464a0d18 crates/oxc_str/src/ident.rs:130-134`, `crates/oxc_str/src/ident_hasher.rs:45-78`; c49e1bd48 (#29397, branch-free ident hash) | c49e1bd48: parser -1.9..-3.8% (lexer +0.2..-5.1%) by fixture; `ident_hash` "was ~6% of parse time on binder.ts" | **No** for ep9 keys. The hash is unseeded and reads at most 16 bytes, so hostile names collide (§6.1, B6) | B6 (refuted) |
| O7 | A CI allocation-count snapshot, and a control row that a change must not move | `oxc-project/oxc@3464a0d18 tasks/track_memory_allocations/` (`allocs_*.yaml`); 9e741c23f (#22842) times against "a parse-invariant control (`estree/checker.ts`)" | 9e741c23f: about 0.5-1% on binder.ts | **Yes**, as bench method | §9 bench plan |

Does not fit a runtime:
- A no-Drop arena with `reset()` for GC objects. GC objects live long and die one by one.
- oxc's unseeded `ident_hash` on property, JSON or Map keys. It breaks H19 G6 (H19:17).
- Removing interning. oxc removed `string-cache` from its AST ("improved the performance of parallel parsing by about 30%", oxc perf page). ep9 atoms are H19 core.
- u32 length fields in arena Vecs, and a stats pre-pass for capacities. Both are compile-only, and PREALLOC already covered capacity.

---

## 2. The hot path B1 removes

```
pop (crates/ep9_builtins/src/array.rs:2290)
 ├─ length_of_array_like  :2299 ─► cx.get(o, "length")  :227
 ├─ (len == 0)            :2303 ─► cx.set(o, "length", 0)
 └─ cx.set(o, "length", newLen)  :2317
        │
        ▼  IntoKey for &str (crates/ep9_vm/src/abi/mod.rs:295-299)
        ▼  Cx::key (abi/mod.rs:681)
        ▼  atoms::intern_str (crates/ep9_vm/src/atoms.rs:409) ─► intern_latin1
        ▼  JsStr::hash_field (crates/ep9_vm/src/string.rs:273-284): seeded SipHash-1-3
        ▼  atom-table probe + equals_text (compare) + memcmp
        ▼  result = the same bits as rt.atom(Atom::Length)
                    (atoms.rs:73; made by init_static, atoms.rs:432-436)
```

Self Ir on this path, CG `head.deltablue.out` (row total 4,599,065,161):

| Function | Ir | Share | Calls |
|---|---:|---:|---:|
| `JsStr::hash_field` | 36,898,964 | 0.802% | 247,047 |
| `atoms::intern_latin1` | 15,347,145 | 0.334% | 247,685 |
| `string::equals_text` | 9,609,717 | 0.209% | (same path) |
| memcmp | 4,793,942 | 0.104% | 251,820 |
| **sum** | **66,649,768** | **1.45%** | |

- `pop` runs 122,900 times on deltablue (CG `head.deltablue.out`, line count of array.rs:2310). That is 2 interns per call.
- CG-jitless `head.deltablue.out` shows the same counts of 4,561,093,633 (1.46%). octane-deltablue: 0.800 + 0.334 + 0.209 + 0.105 = 1.45%.
- On every other row the path is small. earley-boyer: `hash_field` 0.0090% + `intern_latin1` 0.0039%.

---

## 3. ep9 inventory

Security classes used in this report:
- **(a)** ep9's own tables in the JS heap. JS chooses the keys. They hash with the per-process seeded SipHash-1-3 (H19:32, W4-HASH DL:451), or must.
- **(b)** Rust maps keyed by one engine-made value: a dense id (Label, NameId, BindingId, RefId, StringId, IntrinsicId), a heap address, a page index. JS and source text cannot pick the value, only how many there are.
- **(c)** Rust maps keyed by text or numbers that come from source or JS: identifier text, string literals, group names, locale tags, values from the source text.

Std containers by crate. Method: `git grep -nE '(HashMap|HashSet|BTreeMap|BTreeSet)<' m9-p5 -- crates`, `src/` only, test files left out. It counts lines that name the type, so one map can show on 2 lines (a field and a parameter). The gatherer inventory used a wider search; this table was recounted by the report writer at `m9-p5`.

| Crate | Lines | B-tree (no hash) | Hash, class (b) | Hash, class (c) |
|---|---:|---:|---:|---:|
| ep9_frontend | 22 | 5 | 12 | 5 |
| ep9_vm | 17 | 3 | 11 | 3 |
| ep9_rx1 | 5 | 5 | 0 | 0 |
| ep9_regexp | 4 | 0 | 2 | 2 |
| ep9_heap | 4 | 3 | 1 | 0 |
| ep9_facts | 4 | 3 | 1 | 0 |
| ep9_intl | 3 | 1 | 0 | 2 |
| ep9_platform | 2 | 2 | 0 | 0 |
| ep9_bytecode | 2 | 0 | 0 | 2 |
| **total** | **63** | **22** | **27** | **14** |

- Every std hash map in ep9 uses `RandomState` (seeded SipHash), except one: the ephemeron map `EphemeronMap` with the unkeyed `AddrHasher` (crates/ep9_heap/src/trace.rs:181, :198-203), class (b).
- The class (c) lines: bytecode builder.rs:302 (`PendingConst`), :1097 (`UnitString`); frontend bytecodegen/module.rs:323, scope/collect.rs:87-88 (`name_ids`, `private_ids`), surrogate.rs:365, :468; regexp lib.rs:539, :545 (group names); intl locale.rs:532, tag.rs:110; vm facts_export.rs:393 (`ShapeDesc`, made of property names), ic.rs:2015 (a function-name `String` in the miss counters), jobs.rs:187 (`ClosureKey = (usize, u32, String)`, jobs.rs:179; put in (c) by the default rule).

Class (a) tables in the JS heap (ep9's own hash code):

| Table | Hash today | Status |
|---|---|---|
| Atom table (crates/ep9_vm/src/atoms.rs:317-340, :391-436) | seeded SipHash-1-3 (`string::hash`, atoms.rs:331) | OK. B1 and B2 touch only the call path |
| Shape key hash (crates/ep9_vm/src/key.rs:181) | seeded (`string::hash`) | OK |
| `Symbol.for` registry (crates/ep9_vm/src/symbol.rs:466) | seeded (`string::hash`) | OK |
| Map and Set (crates/ep9_vm/src/ordered_table.rs) | strings seeded (:344); numbers use an unkeyed splitmix64 finalizer `mix` (:290-297); BigInt keys use seed 0 (crates/ep9_vm/src/bigint.rs:184) | **G6 defect, already there** (§8) |
| Dictionary-mode elements (crates/ep9_vm/src/elements.rs:528-532) | unkeyed `(i * 0x9E3779B1) >> (32 - bits)`, linear probe | **G6 defect, already there** (§8) |

Only one container path has a profile share above the kill line: the class (a) atom path, through B1.

---

## 4. KEEP (ranked)

| Rank | id | Site | Today | Change | Evidence | Est. gain | Key class | Determinism | Risk | ADR touch |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | **B1** | `intern_str` (crates/ep9_vm/src/atoms.rs:409-415), for all Rust-text keys: `Cx::key` (abi/mod.rs:681) and 50+ VM sites. Hot caller: `pop` (crates/ep9_builtins/src/array.rs:2290-2318), 2 per call | Per call: SipHash-1-3 of the text, a probe, `equals_text`, memcmp | `pub(crate) const fn Atom::from_text(&str) -> Option<Atom>`, made by the `atoms!` macro: match on `len()` first, then byte-slice patterns over the 36 texts. `intern_str` returns `rt.atom(a)` when that root is not UNDEFINED. Before `init_static` (which itself calls `intern_str`, atoms.rs:434) it takes the slow path | §2: 66.6 M = 1.45% of deltablue x64 jit; jitless 1.46%; octane-deltablue 1.45% | deltablue and octane-deltablue: est. **-1.3..-1.4%**, jit and jitless. The match costs est. 10-20 Ir per call (8 of the 36 texts have length 6, like `"length"`), so 2.5-5 M. earley-boyer: <= -0.013%. TS: not measured | none. The key is Rust text compiled into the engine. Other text still takes the unchanged seeded path | Same atom bits: the root is `intern_str(text)` (atoms.rs:434), and `verify()` checks each root is the atom of its text. Table contents, insert order and H13 ids do not change | low. `from_text` must stay `pub(crate)`: `Atom` is in the public-api golden (crates/ep9_vm/tests/golden/public-api.txt:43-47). Every non-static `cx.key` text pays a length-check miss, est. 2-5 Ir per call (for example the digit keys built at array.rs:215-221); check index-heavy rows | H19:35 Interning row: one clarify/fill line, "a static atom's text gives its root". No new atom, so H13:35 `the_atoms_list_is_append_only` is unchanged. `Cx::key` body untouched: no H20 line |
| 2 | **B2** | `atoms::intern` (atoms.rs:317-340) via `intern_key` (crates/ep9_vm/src/key.rs:152-158). Callers: `own_define` (crates/ep9_vm/src/props.rs:404), `add_with` (crates/ep9_vm/src/shape.rs:924), property.rs:798, arguments.rs:712. t0-gen path: `iter_result` (crates/ep9_vm/src/generator.rs:94-99) does 2 `own_define` | An out-of-line call for a key that is already an atom: 28 Ir per call | An `#[inline(always)]` guard (`is_atom` -> return) and an `#[inline(never)]` slow body (oxc O2). Also: `own_define` interns the key, then `add_with` interns the same key again; passing the interned key down removes est. half of the 3.8 M calls on t0-gen, in any form | CG `head.t0-gen.out:15820`: `atoms::intern` 106,437,996 of 2,732,630,205 = 3.90%, 3,801,357 calls. By line: entry atoms.rs:317 38.0 M (10 Ir per call), exit :340 34.2 M (9 per call). The body is the `is_atom` test (crates/ep9_vm/src/string.rs:1013-1021), about 9 Ir per call; it stays at the caller. Prologue plus epilogue: 72.2 M = 2.64%. `own_define` is 4.87% and `add_with` 8.73% of t0-gen | t0-gen est. **-2.6..-2.8%** (call/ret included). Other 19 rows <= 0.002%. TS: not measured | (a), only in the slow path, which does not change | Same atom back; the table is touched in the same order | medium. The guard inlines into `own_define`, `add_with` and `define`. If one of these inlines into a hot T0 handler, a register save can appear (rule 81). FRAME-GOLDEN is the gate | none (H19:35 holds as written) |

B2 is out of the container scope: it is a call shape (CLAUDE.md sort (c)). It goes to the orchestrator as its own micro-item. It is in the same file as B1, so the same writer does it, as its own commit.

---

## 5. WEAK (real, but below the kill line)

| id | Site | Change | Evidence | Est. gain | Key class | Note |
|---|---|---|---|---|---|---|
| B3 (corrected) | Std maps keyed by one engine id in ep9_frontend: bytecodegen/regopt.rs:343-344, :548, :553, :572, :1225 (`Label`); bytecodegen/func.rs:101 (`(FeedbackSlotKind, u32)`: a small enum and one string id); scope/collect.rs:45 (`NameId`), :112 (`RefId`); scope/resolve.rs:481, :525 (`BindingId`) | An in-tree Fx `BuildHasher` (about 20 lines, `BuildHasherDefault`), private to ep9_frontend. No new crate: W4-DEPS-FROZEN (DL:434) lets dependency tables change only in P0. rustc-hash 2.1.3 is in Cargo.lock (Cargo.lock:1266-1267) only through oxc. Leave collect.rs:116 / :911 `heritage_walks` `(u32, u32)` (a span pair) on SipHash: it is two values that source text can steer (§7 rule 1) | CG `head.earley-boyer.out` (4,637,400,404 Ir): `hash_one<&Label>` 1,625,351 Ir (0.035%: 17,861 calls x 91 Ir); Label rehash/insert 0.18 + 0.08 + 0.06 + 0.05 M; `hash_one<(FeedbackSlotKind,u32)>` 0.35 M. Total about 2.4 M = 0.05% | earley-boyer est. -0.03..-0.04% at best; TS est. < 0.01% | (b) | Never iterated into output (regopt.rs:332, :356, :573, :586, :663, :666, :911, :1231, :1243; func.rs:228, :232; resolve.rs:43-312, :481-525; H02:68). `RandomState` already differs per run, so a fixed Fx order adds no nondeterminism. Hostile source cannot pick id values, only how many: a full collision of m keys in a 2^k table needs about m x 2^k ids, so est. O(sqrt(L)) per op and est. O(L^1.5) per compile in the worst case (not measured). That is below the oxc_semantic floor of O(L^2) (§8) |
| B4 | regopt `Label` maps; resolve.rs:481 `param_slot` | A dense `Vec<u32>` by id (oxc O3) | `HashMap<Label,()>::insert` 4,431 and `HashMap<Label,u32>::insert` 5,174 Ir (bench log 44161612861 134316645995:1642, :1646) | <= B3 | (b) | No hash, so nothing to collide; Labels are dense per FunctionBuilder (crates/ep9_bytecode/src/builder.rs:486-488). But `Label(u32)` has a private field (builder.rs:45), so this needs an H02 "API addition" row (`Label::index`). Prefer B3 |
| B5 (safe form) | scope/collect.rs:45 `ScopeExtra.names`: one map and one `RandomState::new` per scope | One flat `(ScopeId, NameId) -> BindingId` std `HashMap` with **one `RandomState` made once per Builder** (seeded SipHash). This takes oxc O4's alloc win without new collisions | `Builder::lookup` 896,291 and `declare_full` 341,575 self Ir (bench log 44161618559 134316665996:1653-1654, jitless octane-earley-boyer) | earley-boyer est. < 0.02% | (b) pair, kept seeded | Never iterated (collect.rs:40-45). The Fx form is refuted (§6.1) |
| B7 | builder.rs:302 `const_index` (`PendingConst`); builder.rs:1097 `StringTable.index` (`UnitString`) | Stays seeded (class (c): string literals). Only remove the key clone: an index-only table | `hash_one<&PendingConst>` 1.38 M + rehash 0.17 M + `equivalent_key` 0.14 M = 1.7 M (0.036%); `hash_one<&UnitString>` 0.63 M (earley-boyer) | < 0.01% | (c) | Needs a hashbrown `HashTable` (a dependency: P0 only) or a std chain. The id order comes from the list (H02:68) |
| B10 (unverifiable) | regexp.rs:170-246 cache; trace.rs:181-197 `AddrHasher`; jobs.rs:187 `ClosureKey` with a `String`; string.rs:1067-1076 index re-hash; dispatch.rs and deopt BTrees | as built | No bench row; the gain verifier did not open each site | 0 (est.) | (b) / (a) | Nothing supports a gain. Do not start these without a profile row |

---

## 6. Refuted (so nobody does them again)

### 6.1 Candidates

| id | What was proposed | Why it is refuted |
|---|---|---|
| B5 as specified | One flat **Fx** map `(ScopeId, NameId) -> BindingId` | Hostile source collides it fully. Identifier references alone make NameIds (collect.rs:520-521 calls `name()` for each `IdentifierReference`), so the attacker makes N names cheaply and puts one chosen name in each of S block scopes. With rustc-hash 2.x, h = rotl(((s*K)+n)*K, 26), and for every s a fitting n exists among about 4 x 2^k names. Simulated in the run: 4096 scopes and 32,768 names gave 4096 of 4096 pairs one h1 in a 2^13 table. hashbrown probes from one home group (hashbrown-0.17.1 src/raw.rs:60-62), so each op walks the whole cluster: O(S^2/16) per compile from source of about O(S log S). eval and `Function` take any source (crates/ep9_frontend/src/eval.rs:184-229). Today each scope has its own SipHash-keyed map, and this attack does not work. Use the safe form (§5) |
| B6 | Key `name_ids` / `private_ids` (collect.rs:87-88) by oxc's precomputed `Ident` hash | The key is identifier text from hostile source. oxc's `ident_hash` takes no seed; for length 17 and up it reads only 4 windows of 4 bytes (oxc_str 0.158.0 `src/ident_hasher.rs:67-74`), and the build hasher adds only the length. At length 20, bytes 4, 5, 10, 11 and 12 are never read: 1024 distinct 20-byte identifiers built in the run all gave one hash, and about 63^5 such names exist. oxc_semantic already uses this hash for its per-scope Bindings (oxc_semantic 0.158.0 `src/scoping.rs:20-21`), but that floor is itself a G6 defect (§8), not a safe baseline. B6 would add a second O(n^2) map that spans the whole program, for est. < 0.01%. Keep SipHash |
| B8 | A faster string hash in place of SipHash-1-3 (string.rs:1201-1290) | After B1, `hash_field` is <= 0.009% on every CG row (earley-boyer is the largest). H19:32 states "SipHash-1-3" as Frozen text, and H12:27 too, so a change counts in G8. Re-check only once TS function rows exist. Any later replacement must keep a per-process secret with no known seed-independent collisions. foldhash does not qualify: its README says it "does not claim to provide HashDoS resistance against interactive attackers" (foldhash-0.2.0 README.md:271-275). V8 derives its rapidhash secrets from the seed (v8 40f4b69c3b src/numbers/hash-seed.cc:34-104) |
| B9 | A bitset for the heap page lists (crates/ep9_heap/src/memory.rs:95-103, `BTreeSet<u32>`) | All `BTree*` functions together are <= 0.0018% on any CG row (octane-raytrace) |
| B11 | Change the shape lookup structure (crates/ep9_vm/src/shape.rs:852-901, a linear scan of up to 8, then an index) | `shape::lookup_with` is 61,492,255 Ir = 2.25% of t0-gen, but half its cost is function entry and exit, and there is no hashing inside. It is a call shape like B2, not a structure change. (The TS 0.67% share was not checked: no TS cachegrind file exists) |
| B12 | Grow the IC field slots differently (`reserve_field_slots`, props.rs) | It already doubles: `need.max(2 * have).max(MIN_SLOTS)` (props.rs:272) |

### 6.2 Claims and fix forms

| Claim | Why it is wrong | Use instead |
|---|---|---|
| "Fx with a seed" fixes HashDoS for B5 or any (c) key | With rustc-hash 2.x add-then-multiply, h(n+d) - h(n) = d*K before the rotate, whatever the seed (rustc-hash-2.1.3 src/lib.rs:97-99) | Seeded SipHash (std `RandomState`) |
| Copy oxc's clippy ban on std maps as-is (`oxc-project/oxc@3464a0d18 .clippy.toml:17-20`) | It pushes new code to Fx everywhere, so it turns into a HashDoS generator for class (c) keys | If ep9 adds a ban, it must exempt class (c) (§7) |
| B1 sum 57.0 M = 1.24% (synthesis) | It left out `string::equals_text`, 9,609,717 Ir (0.209%) on the same path | 66.6 M = 1.45%; est. gain -1.3..-1.4% |
| B1 match costs about 8 Ir | 8 of the 36 texts have length 6 (length, object, number, string, symbol, bigint, return, callee) | est. 10-20 Ir per call |
| B2 keeps 3-4 Ir per call at the caller; est. -3.4% | The guard keeps the full `is_atom` test, about 9 Ir per call (string.rs:1013-1021) | Only the 19 Ir of prologue and epilogue go: est. -2.6..-2.8% |
| B3 Label hash is 0.86 M | That was only the `hash/mod.rs` lines of the fn (CG `head.earley-boyer.out:33650`) | `hash_one<&Label>` 1,625,351 Ir; B3 total about 2.4 M = 0.05% |
| B5 counts at bench log 44161618559 134316665996:1584, :1555 | The values are at :1653-1654 | :1653-1654 |
| oxc keywords.rs:413-438 is a `match` | It is a SIMD perfect hash with a slot table (`keyword_hash_js` plus `js_match_avx2`); its 1% is one hash against another | O1 as written in §1 |

### 6.3 Not repeated from PREALLOC

These were PREALLOC items or PREALLOC refuted forms; BESTFIT does not take them again:
- `names` / `name_ids` reserve by `symbols_len` (refuted: it counts symbols, not distinct names).
- `StringTable.list` reserve (refuted: the slack lives for the unit life).
- vm-rest-8, the `IntrinsicId` set in builtins.rs:1623-1628 (refuted: once per process).
- vm-rest-1 "keep the allocation" in the atom sweep (refuted: the O(cap) re-place stays).
- vm-rest-11 (new atoms for "[object X]"): B1 adds no atom, so it does not touch H13.

---

## 7. The hasher rule for ep9

```
             key comes from ...                         hasher
  ┌───────────────────────────────────────┐
  │ JS values or source text  (a), (c)    │ ─────►  seeded SipHash (per-process key, W4-HASH)
  │ text, numbers, BigInt, spans, names   │         std RandomState in Rust maps
  └───────────────────────────────────────┘
  ┌───────────────────────────────────────┐
  │ ONE engine-made value  (b)            │ ─────►  unkeyed Fx allowed, if rules 1-5 hold
  │ dense id, heap address, page index    │         (else: the seeded default)
  └───────────────────────────────────────┘
  ┌───────────────────────────────────────┐
  │ a fixed set known at build time       │ ─────►  no hash: match or array (B1)
  └───────────────────────────────────────┘
```

The default is a seeded hash: SipHash-1-3 with the per-process key for ep9's own tables (H19:32, H12:27, W4-HASH DL:451), and std `RandomState` for Rust maps. G6 says hostile input must not degrade the engine (H19:17). eval and `Function` make source text hostile input at run time (crates/ep9_frontend/src/eval.rs:184, crates/ep9_frontend/src/function_ctor.rs:164).

An unkeyed fast hash (Fx or similar) is allowed only when all 5 hold:
1. The key is **one** value the engine makes: a dense id from a counter, a heap address, a page index. Not text, not a JS number, not a span. A pair or tuple of two values that source or JS can steer is not allowed (B5 shows why). A small enum plus one id counts as one id.
2. The map is never iterated into output: bytes, order or hashes (H02:68). Or the iteration order is proven not to matter.
3. It is a Rust map. JS-heap tables (class (a)) always stay seeded.
4. No new crate: the hasher is in-tree (W4-DEPS-FROZEN, DL:434).
5. "Add a seed to Fx" is never counted as a mitigation (§6.2).

A dense id with a known bound is better as a `Vec` index than as any hash (oxc O3). A small fixed set is better as a `match` (oxc O1).

---

## 8. Hand back (already in the tree; not speed levers)

| Item | Where | What | Source |
|---|---|---|---|
| HB-1 dictionary elements | crates/ep9_vm/src/elements.rs:528-532 (and :69, :520-550, :796) | Unkeyed multiplicative hash with linear probing. Multiplying by an odd constant mod 2^32 is a bijection, so JS can pick 2^(32-b) indices with one home bucket: checked in the run, 1999 indices, 1 home in a 2^16 table. JS chooses the indices (`o[i] = x`). V8 seeds this table (`NumberDictionaryBaseShape::Hash = ComputeSeededHash(key, seed)`, v8 40f4b69c3b src/objects/dictionary-inl.h:267-268) | security verify |
| HB-2 Map/Set numbers, BigInt | ordered_table.rs:290-297 (used at :352-353); bigint.rs:184; crates/ep9_bigint/src/lib.rs:1028-1040 | Number keys use an unkeyed splitmix64 finalizer, which can be inverted, so JS can pick colliding numbers (est.). BigInt keys use seed 0. W4-H12-Q3 itself says "a fixed seed allows hash flooding" (DL:763) | security verify |
| HB-3 proxy address set | crates/ep9_vm/src/proxy.rs:1084-1101 | A `HashSet<u64>` of `intern_key` bits, which are atom cell addresses. Same pattern as N1, fixed in eval `check_all` (H12:76, :100), so it breaks H12-Q2 (a). Not HashDoS: std `RandomState` is keyed. (The gain verifier did not check the H12 claim) | security verify |
| HB-4 oxc front-end floor | oxc_semantic 0.158.0 `src/scoping.rs:20-21`; oxc_str 0.158.0 `src/ident_hasher.rs:67-74`; ep9 runs `SemanticBuilder` on every script, eval and `Function` source (crates/ep9_frontend/src/lib.rs:414, eval.rs:229) | Per-scope Bindings and UnresolvedReferences use the unseeded `ident_hash`. N colliding `var`s in one scope cost O(N^2). A G6 item, upstream or a pre-check; it does not depend on BESTFIT | security verify |
| HB-5 AddrHasher | crates/ep9_heap/src/trace.rs:198-203 | `(addr >> 3) * K` with no finish mix; hashbrown takes h1 from the low bits (raw.rs:60-62), so ephemeron entries of one size class collide on about log2(size/8) low bits (est.; GC only, mild). A rotate or a finish mix fixes it | security verify |
| HB-6 B2 | §4 | A call-shape micro-item, out of the container scope | gain verify |

---

## 9. Lever proposal: BESTFIT

```
 W-pre ──► role A  BESTFIT-ATOM    (ep9_vm)        B1  (+ B2 as its own commit, if the orchestrator takes it)
           role B  BESTFIT-COMPILE (ep9_frontend)  B3 (corrected) + B5 (safe form)   ← only after the A probe passes
              │
              ├─ step 1 = B1 alone ─► probe ─► deltablue x64 jit Ir down >= 0.2% ?  no ─► STOP the lever
              ├─ step 2 = B2 ──────► probe ─► t0-gen >= 0.2% and FRAME-GOLDEN unchanged ?  no ─► drop B2
              └─ role B ───────────► full bench ─► < 0.2% on every row (expected) ─► hand back: "merge as hygiene?"
```

**Files.** One writer per file.
- Role A:
  - crates/ep9_vm/src/atoms.rs: the `atoms!` macro (`from_text`), `intern_str`, and the `intern` split (B2).
  - crates/ep9_vm/src/tests/strings.rs: the new tests.
  - docs/adr/H19-strings.md: one Change-log row (clarify/fill).
  - B2's "pass the interned key down" part touches props.rs and shape.rs; if taken, the same writer does it in the B2 commit.
- Role B:
  - crates/ep9_frontend/src/fxhash.rs (new) and the `mod` line in `lib.rs`.
  - bytecodegen/regopt.rs, bytecodegen/func.rs.
  - scope/collect.rs, scope/resolve.rs.
  - No ep9_bytecode file: B4 and B7 are left out.

**Bench.** Bench-set: the 19 rows and the TS set, x64 and a64, jit and jitless.
- Function list: `FUNCTIONS=ep9_vm::atoms::intern_latin1,<ep9_vm::string::JsStr>::hash_field,ep9_vm::string::equals_text,ep9_vm::atoms::intern,ep9_builtins::array::pop,__memcmp_avx2_movbe,bcmp`.
- Kill line: >= 0.2% Ir on one row, per item. B1 is judged on deltablue and octane-deltablue (expected est. -1.3..-1.4%). B2 is judged on t0-gen (expected est. -2.6..-2.8%).
- Control row (oxc O7): navier-stokes must stay within ±0.05%.
- Rule 13: no row worse than +0.5%, a64 included.
- Read the index-heavy rows for the B1 length-check miss (est. 2-5 Ir per non-static `cx.key`).
- Read the TS row's `hash_field` and `intern_latin1`. They are not measured yet; this settles B8 later.

**Tests.**
- B1:
  - For every `a` in `Atom::ALL`, `intern_str(rt, a.text())` has the same bits as `rt.atom(a)`.
  - `from_text` gives `None` for `"lengt"` and `"lengthx"`, and `Some(Atom::Empty)` for `""`.
  - A call before `init_static` takes the slow path.
  - test262 `built-ins/Array/prototype/pop`, with and without `--gc-stress`.
- B2: the existing atoms tests under GC stress; FRAME-GOLDEN passes unblessed.
- Role B: `expectations/bytecode-corpus.digest` and the `tests/determinism.rs` `hashes.txt` stay unchanged.

**G8 kind table.**

| Item | ADR touch | Kind | G8 |
|---|---|---|---|
| B1 | H19:35 Interning row: "a static atom's text gives its root" | clarify/fill (same result) | 0 |
| B1 | H13 atom ids, H20 golden, public-api.txt | none (`pub(crate)`, no new atom) | 0 |
| B2 | none | - | 0 |
| B3, B5 | none (H02:68 holds: the maps are never iterated) | - | 0 |
| **total** | | | **0** |

**Rule 81 (interp handlers).**
- `git grep` in interp/ops.rs, tails.rs, dsl.rs and mod.rs for `intern_key`, `atoms::`, `own_define`, `add_with` and `lookup_with` finds only test code in tails.rs (lines 110-130 and 260).
- B1 touches no handler.
- B2 changes inlining in callees that may themselves inline into handlers. Its commit line reads `rule 81: B1 none; B2 indirect, FRAME-GOLDEN must pass unblessed`.
- Role B is compile-only: none.

**Where it fits.** Role A is small: est. 1 agent-day, 2 files and an ADR row. B1 is the only container-side item with a measured share well above the kill line. Role B is the "faster HashMap" part the user asked about. By measurement it cannot reach 0.2% on any row, so it is optional hygiene, and its merge is a product call.
