# ep9 plan: object model, values and spec coverage

- Values keep the JSC NaN-box in `Value(NonZeroU64)` (H6). The value-encoding version is now a unit header field.
- The 8-byte object header keeps the v1 layout. Its u32 field is a ShapeId: an offset into the shape space (H8).
- Only snapshot root shapes get stable ShapeIds. AOT code checks every other shape against an RW cell that the first execution fills [R17].
- Shape constness bits and replacement watchpoints register as InvalidatingFuses (H31). TS hints may only pre-size slots and seed field types, behind guards (A4).
- Strings and numbers stay as in v1. Snapshot atoms get stable indices (H13). All tiers and AOT code share one Math implementation (H36).
- The build-vs-reuse table moves TS lowering into BytecodeGen. It adds the ep9-typecheck oracle and 4 AOT rows.

IDs used here: H3, H5, H6, H8, H13, H16, H17, H18, H19, H22, H31, H32, H34, H35, H36, H37; G7; N5, N6; D4, D9, D20; A0, A1h, A2, A3, A4; M0-M5, M8, M1a; RX0, RX1, RX2; SP-Value, SP-tsrs-win; IntrinsicRef, ShapeRef, FactSnapshot, InvalidatingFuse.

Evidence tags: [R01] [R02] [R05] [R06] [R07] [R08] [R10] [R12] [R13] [R14] [R15] [R16] [R17] [P1]. Paths are in the table at the end.

## 1. Values (H6, unchanged)

| Kind | 64-bit pattern | Test |
|---|---|---|
| Cell pointer | `0x0000_PPPP_PPPP_PPPP`. P < 2^47, 8-byte aligned, inside the cage, never 0 | `(v & NotCellMask) == 0`; NotCellMask = NumberTag \| OtherTag = `0xFFFE_0000_0000_0002` |
| int32 | `0xFFFE_0000_IIII_IIII` | `v >= NumberTag` |
| double | raw f64 bits + 2^49. Top 16 bits run 0x0002-0xFFF2 after NaN purification (JSC allows up to 0xFFFC) | number and not int32 |
| Immediates | null 0x02, false 0x06, true 0x07, undefined 0x0a (as JSC); empty/hole 0x22 = OtherTag 0x2 \| EmptyTag 0x20 (ep9 only). Each has OtherTag set and lies below the first heap page. 0 is never a Value | exact compare; bool `(v & ~1) == 0x06`; undefined or null `(v & ~0x08) == 0x02` |

- `#[repr(transparent)] struct Value(NonZeroU64)`. So `JsResult<Value>` is 8 bytes and returns in one register [P1].
- **Empty/hole = 0x22 (decision H06-empty, 2026-10-20).** JSC uses 0 for empty [R06]. ep9 cannot, because 0 is the niche that keeps `Option<Value>` (None) and `JsResult<Value>` (Err) at 8 bytes. 0x22 fails every other kind test (cell, number, bool, undefined-or-null). It is above JSC's MiscTag 0x0E, so a one-compare `v <= 0x0E` test for bool, null and undefined stays valid. It is not JSC's BigInt32 tag 0x12, NativeCallee tag 0x3 or deleted value 0x4, so those patterns stay free. Costs: a new holey store is filled with 0x22, as V8 fills with the_hole. On aarch64 a hole check is `cmp` + `b.eq`, not one `cbz`.
- **No 0 in a heap slot.** A heap slot typed `Value` never holds 0. `is_cell(0)` is true, so the allocator pre-fills every `Value` slot (undefined, or 0x22 for holey stores) before the next GC point. The release GC visitor still skips 0 and never dereferences it.
- **NaN purification.** Every NaN becomes the one canonical NaN when it is boxed. This covers Float64Array reads, DataView reads and Math results. An impure NaN could otherwise look like an int32 or a pointer.
- **No boxed doubles.** Doubles never allocate. V8 still pays for boxed doubles: its mutable-heap-number work gave up to 2.5x on async-fs [R06].
- **Cost.** Slots are 8 bytes, as in node 24, which has no pointer compression. V8 with compression saves up to 43% of heap [R06].
  - G7 caps both ratios: ≤ 1.2x node --jitless (est.) and ≤ 1.8x a compressed d8 --jitless.
  - N6: no V8-style pointer compression in v1.
- **Encoding version (new).** The unit header records the value-encoding version (H35). On a mismatch the loader ignores the unit's native code and runs its bytecode ([05-aot.md](./05-aot.md)).
- Fix by: M0 week 2 (SP-Value). Full H table: [02-architecture.md](./02-architecture.md).

## 2. Header and objects (H8, extended)

```
word0 (8 bytes, Relaxed atomic):
  bytes 0-3  shape_id  u32   ShapeId: offset into the shape space, O(1) to the Shape; IC guards compare it
  byte  4    gc_state  u8    cell state for the barrier: BLACK=1, WHITE=2, GREY=3 (H11); the mark bit is in side metadata (H9)
  byte  5    kind      u8    GC visitor and size without loading the shape
  bytes 6-7  flags     u16   hash-present, pinned, is-prototype, ...

JSObject:    +0 word0 | +8 properties (out-of-line slots or dictionary) | +16 elements | +24 inline slots[n]
JSFunction:  JSObject fields + SharedFunctionInfo* + Context* + FeedbackCell* + dispatch_index (u32)
SFI:         source range, ScopeInfo, kind flags, name, length, bytecode? (flushable), re-parse data
```

The GC side of word0 (gc_state, kind, forwarding in S3) is owned by [03-gc.md](./03-gc.md) [R05].

### 2.1 ShapeId rule (H8)

| ShapeId kind | Created by | Stable across runs? | In AOT text? | AOT shape guard |
|---|---|---|---|---|
| Snapshot root shape | The startup snapshot, with a deterministic layout (H13) | Yes, per engine build | Yes, as an immediate (ShapeRef) | `header.shape_id == imm` |
| Every other shape | Transitions at run time | No (run-local) | Never | `header.shape_id == RW cell`; the first execution fills the cell |

```
 AOT text (RX, one copy per process)            unit RW data (one copy per isolate, H22)
 ---------------------------------------        -------------------------------------------
 ld   s, [obj + 0]       ; header ShapeId       IcCell[k] = { shape_id, slot }
 ld   c, [rw + k]        ; pc-relative cell       empty at load  -> the first run misses
 cmp  s, c.shape_id                               the IC slow path fills the cell
 b.ne miss               ; H32 policy path        later runs hit; a dead shape clears it (weak)
 ld   v, [obj + c.slot]
```

- This is the Static Hermes model. Its AOT C code keeps per-site `{clazz, slot}` entries in RW arrays: 282 read and 128 write entries for Richards [R14][R17].
- So property-add order, slack tracking and dictionary heuristics need **not** match between build and run. R16 asked for that (R16-D5). R17 shows that RW cells remove the need [R16][R17].
- Deterministic ShapeIds are needed only for snapshot root shapes. That is why the snapshot layout is deterministic (H13).
- The object-layout version is a unit header field (H35). A change rejects the unit's native code; its bytecode still runs.
- Fix by: M0 week 2.

### 2.2 Shapes (H17)

- The same property-add order gives a shared shape through the transition tree [R06].
- The descriptor array is shared along the chain. Each descriptor has a constness bit and a replacement watchpoint set.
- Slack tracking per constructor sets the inline capacity.
- Prototype objects use "prototype mode", with no transition sharing.
- Objects with heavy deletes go to dictionary mode, where ICs do not apply [R06].
- **Fuses (new).** Each constness bit and replacement watchpoint set registers as an InvalidatingFuse (H31). A break lazily deopts dependent code and swaps dispatch slots. The registry lives in `ep9_vm` ([04-execution.md](./04-execution.md)).
- **TS hints (new, A4).** Hints may pre-size inline slots and seed field types. They act only behind a shape or tag guard. Runtime feedback always wins ([05-aot.md](./05-aot.md)).
- **NaN-box note.** V8 runs TS `x;` class fields badly: nbody takes 1184-1428 ms vs 196-201 ms without them. JSC shows no cliff (237 vs 231 ms) [R10][R17]. ep9 uses the JSC encoding. So field-type hints mostly help slot pre-sizing and tag-check removal [R16].
- **Class-field semantics (H3, D20).** `[[Set]]` and `[[Define]]` build different shapes from the same source. 4 of 6 measured codebases compile with `[[Set]]` [R12].
  - BytecodeGen applies the module's flag ([01-front-end.md](./01-front-end.md)).
  - D20 default: follow tsconfig; without one, use `[[Define]]`.
  - The flag sits in the CompiledUnit header, the `.ep9h` header and the cache key.
- Fix by: M1, frozen at the M1a checkpoint.

### 2.3 Elements, keys, realms, contexts, symbols (H18, unchanged)

- **Elements kinds.** PACKED/HOLEY x INT32/DOUBLE/ELEMENTS with one-way transitions. DOUBLE stores raw f64 with a hole NaN. Separate kinds for dictionary, frozen/sealed, typed array, arguments and string wrapper [R06].
- **Property keys.** `PropertyKey = Atom | Symbol | Index(u32)`. Index keys never enter shape tables.
- **Realms.** Each realm has an intrinsics table in a fixed order. Shapes encode the prototype, so ICs stay correct across realms.
  - New: realm intrinsics get stable indices (H13). AOT code reaches them through IntrinsicRef, never through an address (H22).
- **Contexts.** A context is heap-allocated only for captured variables. `with`, direct eval and mapped sloppy `arguments` slow down only the affected function [R07].
- **Symbols.** Heap cells with a "registered" flag, needed for CanBeHeldWeakly [R08].
- Fix by: M1a.

## 3. Strings (H19, unchanged + atom indices)

| Kind | Use | Evidence |
|---|---|---|
| SeqLatin1, SeqUtf16 | Flat storage. UTF-16 only when needed. Lone surrogates force WTF-16 | [R08] |
| Cons (rope), length ≥ 13 | Concat loops. Flattened on index | [R06] |
| Sliced, length ≥ 13 | substring and slice | [R06] |
| Thin | Forwarder after internalization | [R06], [R08] |
| External Latin1 / UTF-16 | Source text, embedder data. Off heap, typed finalizer | [R05] |
| Atom flag + hash field | Interned keys. Cached hash and an "is array index" bit | [R06] |
| Single-char table | 256 prebuilt Latin-1 strings | V8 practice |

- **Rust API.** `JsStr<'s> = Latin1(&[u8]) | Utf16(&[u16])`. Never a Rust `String` as the JS representation.
- **ICU4X.** `icu_casemap` has only `&str` APIs, so ep9 keeps its own Latin-1 case tables. The UTF-16 APIs of the normalizer, collator and segmenter are used directly [R08].
- **Source text.** Kept as an external string. A lazy UTF-8 to UTF-16 offset map serves columns and `Function.prototype.toString` [R01]. Units always carry source (H5, [05-aot.md](./05-aot.md)).
- **Snapshot atoms (new).** Atoms created in the startup snapshot get stable indices (H13). Units name them by index, never by address.
- Fix by: M1a.

## 4. Numbers and BigInt

- int32 fast paths in every tier and code kind. Other numbers are unboxed f64 inside the Value.
- Number to string: `dragonbox_ecma` (0 mismatches on 400k doubles) [R08].
- toFixed, toPrecision, toExponential and toString(radix) need ep9's own exact digit generation. Rust `{:.N}` rounds ties to even, which is wrong for JS [R08].
- Float16 needs its own correctly rounded conversion. The `half` software path gave 15,957 mismatches [R08].
- BigInt: a heap cell with an on-heap digit array. Start with dashu. Port V8 `src/bigint` later, because V8 is 2-4x faster on small numbers [R08].
- **FP rules (H36).** One Rust Math (`ep9_math`) serves T0-T3, both AOT code kinds and all compile-time folds. Its target is the correctly rounded result. `Math.hypot` ports V8; `%` uses an exact Rust `fmod`. No backend may contract to FMA. ToInt32 and IEEE-754 stay exact.
  - Why: Porffor's C backend contracted `a*b+c` to FMA and printed a wrong nbody result [R10][R15].
  - [04-execution.md](./04-execution.md) owns the lint and the differential tests.

## 5. Spec coverage: build vs reuse

| Subsystem | Choice | Crate / version | Effort (PM) | When | Agent fit |
|---|---|---|---|---|---|
| Parser and early errors | Reuse | oxc_parser, oxc_semantic, oxc_regular_expression `=0.155.0` | 1-2 adapter + 0.5-1 upstream | M0 | Medium |
| Scope pass, `CompiledUnit`, BytecodeGen, depth guard | Build | n/a | 4-8 | M0-M1 | Low (human-owned) |
| TS execution (changed) | Build in BytecodeGen (H3). oxc_transformer `=0.155.0` only for legacy decorators (D4); those files get no hints | n/a | 0.5-1.5 (est.) | M4; header fields M0 wk3 | Medium |
| TS type checking (changed) | Reuse out of process: tsrs CLI for `ep9 check` (unix; Windows after SP-tsrs-win), plus the `ep9-typecheck` oracle | tsrs at `tsrs.commit` | 0.25 (+0.25 SP-tsrs-win); oracle 1-2 in A1h (AOT lane) | M4; A1h by M4 exit | Medium (oracle) |
| RegExp RX0 | Fork | regress 0.12.0 fork: Latin-1 cursor, interrupt hook, backtrack budget | 0.5-1 | M1 | High |
| RegExp RX1 | Port | irregexp parser, compiler, bytecode interpreter (BSD-3). Top RegExp tier in AOT-jitless mode | 4-8 | M2-M5 | High (test262 + node diff) |
| RegExp RX2 | Port | irregexp native backend on `ep9_masm`; needs runtime codegen (JIT build only) | 2-4 | M5 | Medium |
| Intl | Reuse | icu 2.3.x baked data + icu_experimental 0.6.0 | 9-15 (less with agents) | M3 | High |
| Temporal | Reuse | temporal_rs 0.2.6 + timezone_provider (jiff-tzdb) | 1-3 glue (Boa glue is 11.7k lines) | M3 | High |
| Date | Build | Own code + port of V8's legacy date parser. Same tz provider as Temporal | 1-2 | M3 | High |
| Number to string | Reuse | dragonbox_ecma 0.1.12 (already in oxc's graph). ryu-js 1.0.3 as the alternative | < 0.25 | M1 | High |
| toFixed, toPrecision, toExponential, radix | Build | Port of V8 fixed-dtoa, bignum-dtoa, DoubleToRadixCString | part of 1 | M1 | High |
| String to number | Build grammar | std `str::parse::<f64>` for the float core | part of 1 | M1 | High |
| Float16 | Build | Own correct rounding (about 40 lines). Not `half` | part of 1 | M3 | High |
| Math (libm-class) | Port | CORE-MATH algorithms in `ep9_math`, seed pxfm 0.1.30 | 1-1.25 | M0 wk3-M1 | High (oracle tests) |
| Math.sumPrecise | Reuse | xsum 0.1.6 | small | M3 | High |
| BigInt | Reuse, then port | dashu 0.6.2, then V8 src/bigint (about 200 KB) on GC digit arrays | 0.5; 1.5-2 | M3; M8 | High |
| Unicode properties, case, normalization | Reuse | icu_properties, icu_casemap, icu_normalizer 2.3 + own Latin-1 tables | 0.5 | M1 | High |
| Strings | Build | n/a | 2-4 | M1 | Low |
| SharedArrayBuffer, Atomics, agents | Build | ecmascript_atomics 0.2.3 approach (inline asm). If vendored, keep its MPL-2.0 files separate | 2-3 | M3 | Medium |
| WeakRef, FinalizationRegistry, WeakMap | Build in the GC | n/a | in GC effort | M0-M1 | Low |
| Proxy, Reflect | Build | n/a | 1-1.5 | M1: core, every internal method with and without traps (M2 gate and the JetStream harness need it); rest of the test262 directories M3 | High |
| Iterator helpers (incl. zip, chunking) | Build | n/a | 1 | M3 | High |
| Explicit Resource Management | Build | n/a | 1.5 | M3 | High |
| ArrayBuffer, TypedArray (resizable, transfer) | Build | n/a | 2-3 | M3 | High |
| Modules (cyclic, TLA, JSON/text, defer, source phase) | Build | n/a | 2-3 | M1-M3 | Medium |
| ValueSerializer (structuredClone, workers) | Build | n/a | 1-1.5 | M4 | Medium |
| Stack traces + V8 API (captureStackTrace, prepareStackTrace, stackTraceLimit) | Build | n/a | 1-2 | M1-M4 | Medium |
| Machine-code encoder | Reuse, then build | dynasmrt 5.1.0, then own encoders. One instruction stream for JIT and unit code (H34) | in M5 | M5 | Low |
| AOT unit writer (new) | Reuse | `object` crate, exact `=` pin chosen at A2, behind `aot` | part of A2 (4-8) | A2, due M5 start | Low |
| AOT unit loader (new) | Build | `ep9_unit` loader (map, verify, register, resolve, fallback) | part of A2 (4-8) | A2, due M5 start | Low (unsafe, human-owned) |
| Hint table schema (new) | Build | `ep9_hints`, zero dependencies (H37) | part of A1h (1-2) | v0 by M4 exit; frozen at A4 start | Medium |
| FactSnapshot and PGO profile format (new) | Build | `ep9_facts`: FactSnapshot schema (= feedback schema, H16), `.ep9prof`, HINTED entries | part of A0 and A3 | Schema frozen M1 exit; `.ep9prof` in A3 | Medium |
| test262 harness | Build | n/a (+ `--force-mode`, [10-quality.md](./10-quality.md)) | 0.5-1 | M0 | Medium |
| TC39 decorators, ShadowRealm, immutable ArrayBuffer | Skip | n/a | 0 | n/a | n/a |

- Effort figures for spec subsystems come from [R08]. AOT rows use the A-stage budgets in [05-aot.md](./05-aot.md); they are not split further.
- "Agent fit: High" means the work has a strong oracle: test262, or a diff against node or V8 tables. AI agents speed such work up a lot. JIT, GC, IC and unit-loader work speed up little [R07].
- The ep9-typecheck facts (99.94% join, 0.87 s for 18 MB) are in [01-front-end.md](./01-front-end.md) and [05-aot.md](./05-aot.md) [R13].

## 6. Do not use

| Do not use | Why | Evidence |
|---|---|---|
| The `regex` crate | No backreferences or lookaround; Nova passes 0/19 lookbehind tests with it | [R08] |
| The `half` crate | Double rounding on x86; 15,957 mismatches in its software path | [R08] |
| malachite, rug | LGPL in a statically linked engine | [R08] |
| Rust fixed-precision formatting for toFixed | Rounds ties to even | [R08] |
| tsrs anywhere in the engine graph | Its `unicode-id-start =1.1.2` pin downgrades oxc to Unicode 15.1 | [R02], [R08] |
| An LLVM or C backend for AOT in v1 (new) | C compilers contract to FMA unless told not to [R10][R15]. It adds a C++ toolchain, which D9 rules out. Cranelift stays the only T3 option (D9, N5) | [R10], [R15] |

## Changes vs plan v1

| Area | v1 | v2 |
|---|---|---|
| Values | NaN-box, NonZeroU64, G7 caps | Unchanged; value-encoding version is a unit header field |
| Header u32 | Shape id, offset into shape space | Named ShapeId (H8). Stable only for snapshot root shapes; AOT guards use RW cells; object-layout version in the unit header |
| Snapshot indices | Intrinsics in fixed order | Stable indices for realm intrinsics, atoms and root shapes (H13); IntrinsicRef in AOT code |
| Shapes | Constness bits, watchpoints | Registered as InvalidatingFuses (H31); TS hints only behind guards (A4); `[[Set]]`/`[[Define]]` per module (H3) |
| Numbers | v1 rules | + one Math implementation and no FMA in every backend (H36) |
| Build vs reuse | TS execution through oxc_transformer; RegExp R0-R2; JIT encoder | TS lowering in BytecodeGen; tsrs CLI + ep9-typecheck; RegExp RX0-RX2 (RX1 is the jitless top RegExp tier); the encoder serves JIT and unit code; 4 new AOT rows |
| Do not use | 5 items | + no LLVM or C backend for AOT in v1 |
| Unchanged | | Elements kinds, keys, contexts, symbols, string kinds, number and BigInt choices |
| Decisions 2026-10-20 | | Value table states the bits; empty/hole = 0x22; no 0 in a heap slot (H06-empty). `ep9_math` with a CR target (H36). Proxy/Reflect core in M1. See [decisions-log §5](../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction) |

## Evidence tags

| Tag | Path |
|---|---|
| [R01] | [../research/01-tsrs-architecture.md](../research/01-tsrs-architecture.md) |
| [R02] | [../research/02-tsrs-conformance.md](../research/02-tsrs-conformance.md) |
| [R05] | [../research/05-gc-design.md](../research/05-gc-design.md) |
| [R06] | [../research/06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R07] | [../research/07-rust-engine-landscape.md](../research/07-rust-engine-landscape.md) |
| [R08] | [../research/08-spec-subsystems.md](../research/08-spec-subsystems.md) |
| [R10] | [../research/10-js-aot-precedents.md](../research/10-js-aot-precedents.md) |
| [R12] | [../research/12-ts-type-soundness.md](../research/12-ts-type-soundness.md) |
| [R13] | [../research/13-tsrs-type-oracle.md](../research/13-tsrs-type-oracle.md) |
| [R14] | [../research/14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R15] | [../research/15-aot-perf-reality.md](../research/15-aot-perf-reality.md) |
| [R16] | [../research/16-aot-synthesis.md](../research/16-aot-synthesis.md) |
| [R17] | [../research/17-aot-refutation.md](../research/17-aot-refutation.md) (wins over [R16]) |
| [P1] | Plan v1 local probe (session-local); re-checked by SP-Value in M0 week 2 |
