# TypedArray design (SPEC-TA, M7-P5)

Status: design text (a plan note), not an ADR. No ADR row comes until the code lands (plan v12 §8.3). This text is the design input of the TypedArray workflow (decisions-log M6-ORCH-321, route TYPEDARRAY-IMPL). That workflow starts after the INTL wave-1 merge (M7-P5 merge 6). It builds ArrayBuffer (with resizable buffers), SharedArrayBuffer, the 12 TypedArray constructors (Float16Array included), %TypedArray%, DataView and Atomics.

- **Cites at 0c58e12f** (tag `m7-p4`, the M7-P5 role base). Every `path:line` names that tree. `docs/plan/notes/` is outside the adr-lint drift scan (`tools/xtask/src/exit/drift.rs:44-45`, "each `*.md` in them, not below"), so a later editor re-checks the cites by hand.
- **Short forms.** Hnn:N = `docs/adr/Hnn-*.md` line N. 13:N = `docs/plan/13-execution-modes.md`. 09:N = `docs/plan/09-milestones.md`. 03:N = `docs/plan/03-gc.md`. AR:N = `docs/adr/README.md`. DL = `docs/decisions-log.md`. Spec = ECMA-262 2025. "node" = node v26.13.0 (each behavior claim below was run there).
- **Engine cites** (read-only clones): V8 at 40f4b69c3bc2; JSC = WebKit at 4ab70932d7cc (paths under `Source/JavaScriptCore/`); SM = Firefox at 7a315fc898bb (paths under `js/src/`); QuickJS at 712e8c0ca269.
- "(est.)" marks an estimate. "TA" = a TypedArray instance. "view" = a TA or a DataView.

## 0. Summary

```text
 new Float64Array(buf, 8, 4)                     the shape of the view (H17 counts word)
   cell kind FLOAT64_VIEW (1 of 12, H17)  ──────  object kind = the cell kind, elements kind 7 TYPED_FIXED
   +24 buffer ─────────────────────────────────►  ArrayBuffer: SLOT_OBJECT, ObjectClass::ArrayBuffer (id 5)
   +32 data (ExternalRef) ──────┐                   slot 0 store: a BYTES cell (on heap) or a table index
   +40 length 4                 │                   slots: byteLength, maxByteLength, detach key, flags, views
   +48 byteOffset 8             └──────────────►  byte 8 of the store
 isolate BufferTable (Rust memory, one weak source): per buffer: owner (weak), off-heap store, views (weak)
 detach / transfer: every live view of the buffer gets data = null and length = 0 (no fuse, no buffer load)
```

| Id | Decision | Why (and who does the same) |
|---|---|---|
| TA-D1 | The element type of a TA is its **cell kind**: 12 new JS-object kind ids, adjacent, in spec Table 71 order. A shape holds the object kind (`crates/ep9_vm/src/shape.rs:31-32`), so one shape guard fixes the element type | 12 types do not fit in the 9 free elements kind values (H18:85); 13:550 and DL M6-PLAN-D5: a typed array fact kind is (cell kind, elements kind) and takes no value. JSC (a `JSType` per type) and SM (a `JSClass` per type) do the same |
| TA-D2 | The **storage form** takes 2 elements kind values: 7 `TYPED_FIXED` (IsTypedArrayFixedLength true: the bound is the view's own length field) and 8 `TYPED_VARIABLE` (false: the bound comes from the buffer). 7 values stay free (9-15) | a shape guard then gives the bound source too, and keyed feedback can tell a view from an Array. V8 splits the same way (RAB_GSAB kinds), SM too (fixed-length and resizable classes). Fallback: §3.4 |
| TA-D3 | ArrayBuffer, SharedArrayBuffer and DataView are **SLOT_OBJECT classes** (ArrayBuffer is declared already, `crates/ep9_vm/src/abi/slots.rs:54-57`; append SharedArrayBuffer and DataView). No new cell kind for them | the brand checks are `object_class` tests; no hot path reads a buffer of a fixed-length view |
| TA-D4 | The bytes of a **fixed-length, non-shared** buffer live **on the heap** in a new BYTES cell (small cell or LOS). Resizable buffers and all SharedArrayBuffers live **off the heap** in a reserved region; the isolate BufferTable owns them and frees them in its weak source | no Rust code runs when a cell dies (`crates/ep9_heap/src/lib.rs:33`); a heap cell needs no free hook and counts against `max_heap_bytes`. The RegExp program table is the precedent for off-heap data (`crates/ep9_vm/src/regexp.rs:315-360`) |
| TA-D5 | A view caches its data address and length. Detach and transfer walk the buffer's weak view list and zero each view | the fast path then needs no detach check and no fuse. JSC (`runtime/ArrayBuffer.cpp:532-540`), SM (`vm/ArrayBufferObject.cpp:1299-1325`) and QuickJS (`quickjs.c:60927-60939`) do this; V8 tracks one view, then pops a protector (`src/objects/js-array-buffer.cc:197-243`) |
| TA-D6 | The view kinds get their own range compare. `property::own_index` refuses them (`crates/ep9_vm/src/exotic.rs:261-264`, second option); the routed range 25..=28 stays closed (`crates/ep9_vm/src/kinds.rs:44-45`) | H31:56 row 3; no renumber |
| TA-D7 | A view that becomes a prototype pops "no-elements prototypes" at any length (MC-w26-4) | §4.2 |
| TA-D8 | The T0 typed fast cases go in `keyed_ic.rs`. No `interp/ops.rs` change, no handler ABI change, no dispatch-core change (plan rule 72). BigInt views stay cold | "a hit never allocates and never calls" (`crates/ep9_vm/src/keyed_ic.rs:65`) |
| TA-D9 | No new IC handler kind in month 8. The CacheIR op list gets appends for views (H16:146). The keyed kinds-set bits 17 and 18 are in IC_LAYOUT 4 already (H18:85) | no IC_LAYOUT bump |
| TA-D10 | FactSnapshot: `ObjectKind::TypedArray` (8, declared) gets its writer; ElementsKind names 7 and 8 are appended (outside the fingerprint). No FACT_SCHEMA bump in month 8. [2026-11-10, TA-0 fixer, clarify, TA-ORCH-1 (M6-ORCH-366)] The names 7 and 8 are not a fill in TA-0. With the names and no writer, `every_field_that_claims_a_writer_is_seen` (`crates/ep9_builtins/tests/facts_export.rs`) fails for 6 items (`KeyedFacts.kinds`, `LiteralFacts.elements` and `ShapeDesc.elements`, each with typed-fixed and typed-variable), and H16:237 lets an item join the declared-no-writer list only with a FACT_SCHEMA bump (month 8 has none). So the names 7 and 8 and the NO_WRITER 17 -> 16 move land in TA-3 (TA-CTORS), the first phase whose JS corpus makes a view; TA-2 TA-KEYED keeps NO_WRITER at 17. Until then `elements::to_fact` gives None for 7 and 8. | H18:88, H16:237 |
| TA-D11 | Shared bytes: every Rust access is atomic (Relaxed for plain element access, SeqCst for `Atomics`). One process waiter list, keyed by (block serial, byte index). `$262.agent` agents are isolates on their own threads | H22:16 (an isolate per thread); Rust has no race-free plain access to shared memory |
| TA-D12 | Float16 conversion goes through `ep9_math` (one rounding, ties to even), as `Math.f16round` does (`crates/ep9_builtins/src/math.rs:334`; H36:45) | H36: one Rust math crate for all tiers |
| TA-D13 | OBJECT_LAYOUT 8 -> 9 once, in phase TA-0, by one writer; no ABI, IC_LAYOUT, DEOPT_META, VM_LAYOUT or FACT_SCHEMA move | plan rule 2 form; §8.3 |
| TA-D14 | The TA globals (16 names) are one lazy namespace group (the M7-P1 LAZYNS form) | start-up cost |

## 1. Scope and test262 counts

**Source.** CI run 42609805000, linux-x64 `test262-exec.json` and `test262-exec-eager.json` (the m7-p4 engine; jit preset; test262 at d53ebc8b). Both files have the same 7,555 fail paths. The TA-family rule is the pre-step classifier rule (path under one of the 6 TA directories of `built-ins`, or a TA feature tag, or a "TypedArray / ArrayBuffer / DataView / SAB / Atomics is not defined" cause).

**Result: 3,874 TA-family fails** (not 3,877; see the role report). The other classes: Intl 3,167, N3 93, staging 38, left 383.

| Directory | Files | Fails | TA-family fails | Pass now |
|---|---|---|---|---|
| built-ins/TypedArray | 1,453 | 1,453 | 1,453 | 0 |
| built-ins/TypedArrayConstructors | 738 | 738 | 738 | 0 |
| built-ins/DataView | 561 | 561 | 561 | 0 |
| built-ins/Atomics | 389 | 389 | 389 | 0 |
| built-ins/ArrayBuffer | 221 | 221 | 221 | 0 |
| built-ins/SharedArrayBuffer | 104 | 104 | 104 | 0 |
| built-ins/Uint8Array (base64, hex) | 70 | 70 | 70 | 0 |
| staging | 1,483 | 191 | 151 | 1,292 |
| built-ins/Array | 3,083 | 88 | 88 | 2,995 |
| language/statements | 9,347 | 80 | 45 | 9,267 |
| built-ins/Object | 3,411 | 19 | 18 | 3,392 |
| language/expressions | 11,102 | 225 | 14 | 10,877 |
| built-ins/ArrayIteratorPrototype | 27 | 10 | 10 | 17 |
| harness | 116 | 5 | 5 | 111 |
| built-ins/Temporal | 4,605 | 2 | 2 | 4,603 |
| 5 more directories (annexB TypedArrayConstructors, Function, Map, Set, language/destructuring) | - | 5 | 5 | - |
| **all** | 53,616 | 7,555 | **3,874** | 46,061 |

| Feature tag (a file can have more than one) | Files | | Need of the file | Files |
|---|---|---|---|---|
| TypedArray | 2,530 | | includes `testTypedArray.js` | 2,086 |
| resizable-arraybuffer | 471 | | names `SharedArrayBuffer` | 579 |
| SharedArrayBuffer | 467 | | resizable (`maxByteLength` or the tag) | 475 |
| Atomics | 386 | | detach (`detachArrayBuffer.js` or `$262.detachArrayBuffer`) | 335 |
| ArrayBuffer | 278 | | names `Atomics.wait`, `notify` or `waitAsync` | 230 |
| DataView | 193 | | names BigInt64Array or BigUint64Array | 183 |
| Atomics.waitAsync | 101 | | `$262.agent` or `atomicsHelper.js` | 112 |
| uint8array-base64 | 71 | | flag `async` | 66 |
| arraybuffer-transfer | 59 | | names Float16Array | 56 |
| Float16Array | 46 | | flag CanBlockIsTrue / CanBlockIsFalse | 7 / 2 |

So the runner needs two new host hooks: `$262.detachArrayBuffer` (335 files) and `$262.agent` (112 files). Today the `$262` object has neither (`tools/test262/src/exec.rs:50-56`; H22:46). The runner does not read the CanBlock flags.

**Target (est.).** After the workflow: ≥ 3,600 of the 3,874 files pass (est. 93%). The rest (est.): waitAsync timing files, `staging/sm` files that need other shell functions, and a few files that also need Intl or a later feature. M3 asks for the full suite ≥ 97.5% (09:118), and its scope names TypedArrays, SAB, Atomics and agents (09:116).

## 2. Cells and objects (H17)

### 2.1 Kind ids

| Item | Rule at the base | Design |
|---|---|---|
| JS object ids | 16-63; 29-63 reserved (`crates/ep9_vm/src/kinds.rs:11`) | 12 view kinds, adjacent, at the next 12 free ids of the TA-0 base (29-40 if no other role takes an id first). Order = spec Table 71: Int8, Uint8, Uint8Clamped, Int16, Uint16, Int32, Uint32, BigInt64, BigUint64, Float16, Float32, Float64. Then "BigInt view" is one range test of 2 ids and "float view" one range test of 3 ids |
| internal cell ids | 64-127; 85 is the next free (`crates/ep9_vm/src/kinds.rs:12`, PROPS_INDEXED 84) | BYTES (raw bytes): the next free id (85 if free) |
| place in the file | new ids go after SHAPE, so cited lines do not move (`crates/ep9_vm/src/kinds.rs:155`; BIGINT 9 is at `:263`) | the new consts go at the end of the const block, after BIGINT |
| routed kinds | one range 25..=28, closed; a new routed kind gets its own compare (`crates/ep9_vm/src/kinds.rs:44-45`) | `is_view_kind(k)` = one range compare; `property::own_index` (`crates/ep9_vm/src/property.rs:1983`) refuses it, so the keyless get and `in` paths send a view to its hooks (`crates/ep9_vm/src/exotic.rs:261-264`) |
| kinds that keep index keys at +16 | `elements::plain_elements_kind` (`crates/ep9_vm/src/elements.rs:251`) | not the view kinds: a view never stores an index key. SLOT_OBJECT is in the list, so a DataView and a buffer keep ordinary index properties (node: `dv[0] = 1` makes a property) |

KIND (AR:75): an id append is a fill of H17 "kind ids (append-only)" (H17:177). The precedent is BIGINT 9 (H17:269: `kinds::tests::table_v1_ranges` count 43 -> 44 and `tests::layout3::kind_ids_are_an_append_only_byte_list` get the new ids as part of the fill).

### 2.2 Layouts (OBJECT_LAYOUT 9)

```text
 view cell (each of the 12 kinds)          BYTES cell (internal)            ArrayBuffer / SAB (SLOT_OBJECT)
 +0  word0 (ShapeId, kind)                 +0  word0                        +24 class id (5, or SAB)
 +8  SLOTS / PROPS                         +8  byte length (number Value)   +32 slots cell:
 +16 ELEMENTS: always undefined            +16 the bytes, zeroed at birth       0 store: BYTES cell | table index | undefined
 +24 buffer (Value, strong)                    (8-aligned; no Value;            1 byteLength (number)
 +32 data: ExternalRef<u8> (native word)       the GC reads no word)            2 maxByteLength (undefined: fixed)
     = store base + byteOffset; null                                            3 detach key (undefined by default)
     when detached or 0 bytes                                                   4 flags (int32): detached, resizable
 +40 length: int32 Value < 2^31, double                                         5 table index of the view list, or undefined
     from 2^31 (the ARRAY +24 form);
     0 when detached; TYPED_VARIABLE:      DataView (SLOT_OBJECT, class DataView): 0 buffer, 1 byteOffset,
     the fixed length or the "auto" int32 -1    2 byteLength or "auto", 3 data (ExternalRef), 4 flags
 +48 byteOffset (number Value)
 +56 in-object property slots
```

- The view fields start at `obj::FIXED` +24 (`crates/ep9_vm/src/object.rs:82-89`). The length form copies ARRAY +24 (`crates/ep9_vm/src/object.rs:278-279`; H18:84), so one int32 compare bounds an int32 key.
- Every word after word0 is a Value or a native word (a valid non-cell Value). So the view kinds use the common object trace, and the GC never follows `data` (H25:26, H25:61).
- New H25 inventory rows: the view `+32` and the DataView slot 3. Each row is a fill (H25:61). [2026-11-11, TA-1 fixer r3 (Codex TA-1 r1 finding 1; design study `m2/ta/ta1/design/judge.md` 3.4): DataView slot 4 is no flags word: it is `dv_slot::TABLE`, the int32 index of the DataView's own BufferTable entry (its brand, `array_buffer::is_data_view`); the count stays 5 and auto length stays in slot 2. Buffer slot 5 is always the buffer's own entry index (never undefined).]
- A BYTES cell above 8 KiB goes to the LOS (`crates/ep9_heap/src/layout.rs:19`). The cage is 64 GiB (`crates/ep9_platform/src/va.rs:49`).
- **Layout facts for native code** (plan rule 71): `ep9_vm::layout` exports, owned by OBJECT_LAYOUT 9 (`crates/ep9_vm/src/layout.rs:248`): the view field offsets, the first and last view kind id, the element size table, the BYTES data offset, and the elements kind values 7 and 8.
- KIND: H17 has no part for these layouts. A new Freeze-table part "view cells" with its own freeze point (est. the TA workflow merge) is pre-freeze churn, so G8 does not count it. The precedent is the "BigInt cell" part (H17:187, H17:270). The OBJECT_LAYOUT row is "rewrite (version)" on the open parts (H17:271 form). The layout core text (H17:176) does not change. [2026-11-10, TA-0 fixer, clarify] The OBJECT_LAYOUT number in the H17 layout-core What and Check cells moves 8 -> 9 (H17 clarify row 2026-11-10, item (2)); no layout-core fact changes.

### 2.3 Where the bytes live

| Buffer | Store | Freed by | Moves? |
|---|---|---|---|
| fixed-length ArrayBuffer | BYTES cell in slot 0 | the GC (a dead cell) | never (non-moving heap; S3 must pin BYTES, §5.5) |
| zero-length or detached | none (slot 0 undefined) | - | - |
| resizable ArrayBuffer | an off-heap `Reservation` of `maxByteLength` (`crates/ep9_platform/src/va.rs:88`); `resize` commits or decommits pages (`:185`, `:199`) | the BufferTable weak source, when the buffer dies | never: the reservation keeps its base, so cached view addresses stay valid |
| SharedArrayBuffer (fixed or growable) | an off-heap block in an `Arc`, reserved to `maxByteLength`; one `Arc` per isolate that holds the SAB | the last `Arc` drop (each isolate's weak source drops its own) | never |

Size limits (est.): a fixed buffer can be as large as the heap gives; a failed allocation is a RangeError (spec CreateByteDataBlock). A `maxByteLength` above 2^32 (est.) is a RangeError at reservation time. Both caps are T0 tuning facts, not rules for other tiers.

### 2.4 ObjectClass appends (H20 part 2)

`SharedArrayBuffer` and `DataView` append to `ObjectClass` after the INTL wave-1 classes (route INTL-OBJCLASS-RENUMBER moves INTL to the ids after 25). Each append is a fill of H20 part 2 (H20:89) with appended lines in `crates/ep9_vm/tests/golden/object-classes.txt`. `ObjectClass::ArrayBuffer` (id 5) gets its first maker.

## 3. Elements kinds (H18)

### 3.1 The re-check of H18:85

H18:85 holds at the base: the values are PACKED_INT32 0 ... DICTIONARY 6 (`crates/ep9_vm/src/elements.rs:88-101`); 7-15 are open; "a new kind ... takes the next free value; no value is renumbered or reused"; "An append is a fill ... G8 never counts an append". Today `elements::storage` (`:116`) and `elements::to_fact` (`:132`) give none for 7-15, and `storage_form_ok` refuses them (`:720`). Each value already has its keyed kinds-set bit 10 + value in IC_LAYOUT 4 (H18:85; `crates/ep9_vm/src/ic_ops.rs:368-371`). H18:87 keeps 7-15 for storage forms ("F7 typed storage, an undefined-in-double form").

### 3.2 Proposal A (recommended): 2 values

| Value | Name | Storage form | Bound | Example |
|---|---|---|---|---|
| 7 | TYPED_FIXED | the view's bytes (+16 undefined); the element type is the cell kind | `+40` of the view | `new Int8Array(8)`; a fixed-length view on a growable SAB (it can only grow) |
| 8 | TYPED_VARIABLE | as 7 | computed from the buffer: `(byteLength - byteOffset) / size` for an "auto" view; the fixed length, or out of bounds, for a fixed view on a resizable buffer | `new Int8Array(new ArrayBuffer(4, {maxByteLength: 8}))` |

- The value follows the spec test IsTypedArrayFixedLength: 7 when it is true, 8 when it is false. node: `Reflect.preventExtensions` of a length-tracking view gives false, which follows the same test.
- A view gets its value at birth and keeps it. No transition reaches 7 or 8 and none leaves them: `join` and `transition` never see a view (the one-way rule holds with no new edge). 7 and 8 are no birth kind of an allocation site (`generalize_site` stops at DICTIONARY, H18:87).
- `storage_form_ok`: for 7 and 8, +16 is undefined and the cell kind is a view kind.
- FactSnapshot names: "typed-fixed" (7) and "typed-variable" (8), appended to the `ElementsKind` and `ElementsKindSet` name lines of `schema.txt` (outside the fingerprint; H18:85 note, H18:88). A name must not start with "holey" (`crates/ep9_vm/src/tests/elements.rs:803-808` checks the holey names). [2026-11-10, TA-0 fixer, clarify, TA-ORCH-1 (M6-ORCH-366)] The names 7 and 8 are not a fill in TA-0. With the names and no writer, `every_field_that_claims_a_writer_is_seen` (`crates/ep9_builtins/tests/facts_export.rs`) fails for 6 items (`KeyedFacts.kinds`, `LiteralFacts.elements` and `ShapeDesc.elements`, each with typed-fixed and typed-variable), and H16:237 lets an item join the declared-no-writer list only with a FACT_SCHEMA bump (month 8 has none). So the names 7 and 8 and the NO_WRITER 17 -> 16 move land in TA-3 (TA-CTORS), the first phase whose JS corpus makes a view; TA-2 TA-KEYED keeps NO_WRITER at 17. Until then `elements::to_fact` gives None for 7 and 8.

**KIND test (AR:75, AR:76).** Every existing check keeps its result:

| Check | Result after the append |
|---|---|
| `tests::layout5::elements_kind_values_and_storage_forms` | reads only 0-6 (`crates/ep9_vm/src/tests/layout5.rs:100-101`): same |
| `tests::layout5::h18_kind_value_probe_15` | probes 15, never 7 (`crates/ep9_vm/src/tests/layout5.rs:147`): same |
| `tests::elements::the_fact_kind_map_is_one_to_one` | its "reserved VM kind" case for 7 and 8 becomes a named case: part of the append's fill (H18:85 note) [2026-11-10, TA-0 fixer, TA-ORCH-1: in TA-0 to TA-2, 7 and 8 keep the "reserved or unnamed" case; the named case comes with the names in TA-3] |
| `elements.rs:1056-1057` build asserts | `COUNT + 10 <= 31` holds for COUNT 9 |
| the IC_LAYOUT guard (mask `0xFFFF << 10`) | same (bits 17, 18 are inside) |
| Spike C `CheckArray` (`spikes/spike-c/src/vm_layout.rs:20`, `:59`) | exits on 7 and 8 as on any non-array kind: same |

13:550 asks for an orchestrator decision row for each new value that states how many remain. **The row comes before TA-0: "values 7 and 8 taken; 7 remain (9-15)".**

### 3.3 Why not one value per element type

12 types (24 with the resizable forms, as V8 has them: `src/objects/elements-kind.h:19-31`, `:35-48`) do not fit in 9 values. 13:550 and DL M6-PLAN-D5 already say that the fact kind of a typed array takes no value. JSC keeps the type in `JSType` (`runtime/JSType.h:100-112`) and the length mode in the view (`runtime/JSArrayBufferView.h:59-104`). SM keeps both in the class (`vm/TypedArrayObject.h:53-63`).

### 3.4 Fallback B: 0 values

If the orchestrator reads 13:550 as "no value for a typed array at all": the view shapes keep kind 0, `storage_form_ok` gets a cell-kind arm (the ARGUMENTS form, `crates/ep9_vm/src/elements.rs:720-723`), and the fixed/variable bit goes to the view's flags. Costs: one more load and test on every typed access, and keyed feedback records a view as PACKED_INT32, so a site that sees Arrays and views mixes them. Proposal A has neither cost.

### 3.5 Not used

The H18 "extra element kinds" levels (SEALED, FROZEN) are for Arrays only (H18:87). A view never gets a level: node gives a TypeError for `Object.freeze` of a non-empty view, and `Reflect.defineProperty(ta, 0, {configurable: false})` gives false.

## 4. Element semantics (spec §10.4.5, checked on node)

### 4.1 Rules every path keeps

| Rule | Spec | node result |
|---|---|---|
| A canonical numeric string key ("-0", "1.5", "Infinity", "NaN") is a view key, not a property key: no prototype walk, no own property | §10.4.5.1-7, CanonicalNumericIndexString | `ta["-0"]`, `ta["1.5"]`, `ta.Infinity` give undefined with these names on `Object.prototype`; `ta["-0"] = 5` defines nothing; `ta["01"] = 5` is an ordinary key |
| [[Set]] converts the value (ToNumber or ToBigInt) before the bound check; the conversion can detach or resize | TypedArraySetElement | `ta[5] = {valueOf}` calls valueOf once (out of bounds); a valueOf that detaches gives length 0 and no store |
| An out-of-bounds load is undefined with no prototype walk | §10.4.5.4 | `Object.prototype[9]` is not seen through `ta[9]` |
| An element is {w, e, c} | §10.4.5.1 | `getOwnPropertyDescriptor(ta, 0)` = writable, enumerable, configurable |
| Uint8Clamped rounds half to even and clamps | ToUint8Clamp | `[1.5, 2.5, -1, 300, NaN, 254.5]` -> `[2, 2, 0, 255, 0, 254]` |
| Float32, Float16 round to nearest, ties to even | NumericToRawBytes | Float16 of 1/3 = 0.333251953125; 65520 -> Infinity |
| Uint32 above 2^31 - 1 loads as a double Number | RawBytesToNumeric | 2147483653 |
| A NaN store writes one NaN pattern | NumericToRawBytes ("any implementation chosen NaN") | node writes 0x7FF8_0000_0000_0000; ep9 boxes every NaN as that pattern (H36:121), so a store writes it too |
| BigInt64 and BigUint64 take only a BigInt | ToBigInt | `a[0] = 1` throws TypeError |
| Detach makes length, byteLength and byteOffset 0 | IsTypedArrayOutOfBounds | after `buffer.transfer()`: `[0, 0, 0]`, `ta[0]` undefined |
| A shrink puts a fixed view out of bounds; an "auto" view follows | resizable buffers | `resize(4)` of an 8-byte buffer: fixed view length 0, auto view length 4 |

### 4.2 A view as a prototype (MC-w26-4)

[[Set]] of a view with a receiver that is not the view returns true for an invalid index and stores nothing (§10.4.5.5 step 1.b.ii). node: `o = Object.create(new Int8Array(0)); o[0] = 1` leaves `o` with no own "0". So a view of **any length**, even 0, changes the store of every index on objects below it. The no-elements fuse (`crates/ep9_vm/src/shape.rs:1460-1462`) must pop when a view becomes a prototype. Fix: `may_have_index_keys` (`crates/ep9_vm/src/shape.rs:1481-1488`) returns true for a view kind, beside the routed-kind arm. The test is in §9.1.

## 5. GC, detach and memory rules (plan rule 8)

### 5.1 What holds what

```text
 view ──+24 strong──► buffer ──slot 0 strong──► BYTES cell            (on heap: the GC frees it)
                         └──slot 5 int32──► BufferTable entry i        (Rust memory)
                                              owner: the buffer       (weak)
                                              store: Reservation | Arc<SharedBlock> | none
                                              views: [view, ...]      (weak)
 BufferTable weak source (every GC kind, minor too): owner dead -> drop the store, free entry i;
                                                     else drop the dead views from the list
```

- Rust memory holds a Value only weakly, through one weak source that every GC kind runs (`crates/ep9_heap/src/heap.rs:1344-1348`; the minor GC runs weak sources: `crates/ep9_heap/src/tests/minor.rs:277`). This is plan rule 8.
- A live view holds its buffer, and the buffer holds its store. So `data` is valid while the view is live.
- The table is indexed by an int32 that the buffer holds, never by an address (the H12 rule; the RegExp table form, `crates/ep9_vm/src/regexp.rs:315-360`).
- An entry exists only for an off-heap store or a buffer with a view. A buffer with no view and an on-heap store has no entry. [2026-11-11, TA-1 fixer r3 (Codex TA-1 r1 finding 1; design study `m2/ta/ta1/design/judge.md`, `brand.md`; M6-ORCH-407): replaced. Every engine buffer and DataView (one that `array_buffer.rs` makes) owns one entry from birth (buffer slot 5, DataView slot 4); the owner is the brand (`buffer_class(rt, ·)`, `is_data_view(rt, ·)`), and only `BufferTable::insert` sets an owner. Only `array_buffer.rs` writes their slots (`object::engine_slot_set`); `object::slot_set`, the one writer path of the slots cell, panics on an engine object, and a slot access of a value that is not an object with internal slots panics in every build. Rules for TA-2 to TA-4 (judge 4.7): (1) a raw access trusts the slots only of an object that passed `buffer_class(rt, ·)` or `is_data_view(rt, ·)`; `object_class(v) == Some(ArrayBuffer)` is no buffer test, and `cx.slot` is no getter (it also reads caller objects); (2) TA-2 DataView get and set: `is_data_view` first (else a TypeError), then IsViewOutOfBounds from slots 1 and 2 and the buffer's byte length, then slot 3 (or `data_base` + offset; TA-2 decides, R2); (3) the TA-2 getters (`byteLength`, `maxByteLength`, `resizable`, `detached`; DataView `buffer`, `byteOffset`, `byteLength`) are brand-checked `abi/typed.rs` methods (part-2 additions, fill); (4) every new buffer (a resize result, transfer, a `$262.agent.broadcast` SAB in another isolate, structuredClone later) comes from `new_buffer_object`, so it owns its entry at birth, and every slot write of these objects is `engine_slot_set` in `array_buffer.rs`; (5) TA-3 adds a BENCH-SET row that makes many small buffers (R4: one entry per buffer); (6) a JIT inline of DataView or buffer accessors cannot inline the table test (R6: a buffer or DataView kind in the H17 "view cells" part before its freeze point). Known gap: route SAFE-COLL-FORGE (collection forging from safe calls, pre-existing since M3-P1) can still write raw cell words; the TA-1 memory-safety claim excludes it until that route lands.]
- Bound: the list holds only views that were live at the last GC. It is not a cache: it never refuses work.
- An unmap of a dead reservation runs after the GC ends, not inside the weak source (est. cost: one syscall per dead resizable buffer).

### 5.2 Detach, transfer, resize

| Event | Order of the steps |
|---|---|
| DetachArrayBuffer(buffer, key) (`$262.detachArrayBuffer`, `transfer`, `transferToFixedLength`) | 1 check the key and that the buffer is not shared; 2 for each live view: `data` = null, `length` = 0 (a DataView: slot 3 null, byteLength 0); 3 set the detached flag, byteLength 0; 4 then release the store (a BYTES cell: drop the slot; off heap: drop it from the entry) |
| `transfer(n)` | as detach, but the new buffer takes the old store (a BYTES cell of the same length moves with no copy; else copy `min(old, n)` bytes) |
| `resize(n)` of a resizable buffer | commit or decommit pages, then set byteLength. No view field changes: TYPED_VARIABLE views compute their bound at each access, and the base address does not move |
| `grow(n)` of a growable SAB | commit pages, then a SeqCst store of the new length in the shared block. Views in other isolates read the length atomically |

So a fast path that passes its bound check reads valid memory: a detached view has length 0, and a TYPED_VARIABLE path reads the current byteLength first.

### 5.3 GC rules for the cells

- BYTES: the trace function visits no word (as ELEMENTS_DOUBLE). A store into bytes takes no write barrier: the bytes are not Values.
- View cells: the common object trace; `data` is a native word, so the GC skips it.
- A new cell is zero-filled before the next GC point (H06 "no 0 in a Value slot" holds: the zeroed bytes are not Value slots).
- GC stress: every new test runs under `--gc-stress 1` too (H18 Rules form).

### 5.4 Off-heap pressure

The heap has no external-bytes input today (`crates/ep9_heap/src/heap.rs:105`, `max_heap_bytes` only). A loop that makes resizable buffers or SABs could grow off-heap memory with no GC. Phase TA-0 adds one heap API: the BufferTable reports committed off-heap bytes, and the heap counts them in its next-GC trigger (a GC role grant, §8). [2026-11-10, TA-0 fixer, clarify, TA-ORCH-2 (M6-ORCH-366)] As built in TA-0: `Heap::add_external_bytes`, `Heap::remove_external_bytes` and `Heap::external_bytes` (`crates/ep9_heap/src/heap.rs`). The reported bytes count only in the next-GC trigger, whose bound is at most the room that `max_heap_bytes` leaves next to the committed heap. An add never fails for `max_heap_bytes`: month 8 has no admission check of external bytes (the Frozen H23 rows H23:36, H23:37 and H23:56 keep their text; G8 0). Route TA-EXTBYTES-ADMIT (c), due M2 exit. [2026-11-10, TA-0 fixer, clarify] The caller contract of `Heap::add_external_bytes`: do not hold a borrow of a weak source's state across this call (it is a GC point). Put the store in the table only after Ok; on Err, free the store.

### 5.5 S3 evacuation

S3 (03:152) moves cells of sparse blocks. A view caches a raw address into a BYTES cell. Rule: S3 treats a block that holds a BYTES cell as pinned (as CSS-pinned cells). The other form, a Value base plus an offset per access (V8: `src/objects/js-array-buffer.h:515-530`), costs one add on each access; take it only if S3 data shows the pins cost RSS.

### 5.6 Shared memory and threads

- An isolate runs on one thread; only SAB memory is shared (H22:16, H22:32; H22-Q8 at H22:92 is the fill point).
- Rust has no race-free plain access to memory that another thread writes. So every Rust access to SAB bytes is atomic: Relaxed loads and stores of the element width for element access and for bulk copies (aligned: a plain `ldr`/`mov` on aarch64 and x86-64), SeqCst for `Atomics`. A DataView access to SAB bytes is a Relaxed byte loop. Non-shared bytes use plain access.
- Mixed-size atomic access (an Int8Array and an Int32Array on one SAB) is defined in JS but not in the Rust model. Miri skips the SAB tests (`cfg(miri)`); the hardware gives single-copy atomicity for aligned accesses up to 8 bytes. [2026-11-11, TA-2 fixer r3 (Codex TA-2 r2 finding 1; TA-ORCH-12, M6-ORCH-443), as built: N1 lets two isolates on two threads share one block, so a mixed-size access between two plain accesses became reachable (a DataView `setInt32` and a `setUint8` on the same bytes). Now every plain (Unordered, ECMA-262 §29) access to SAB bytes is a loop of one-byte Relaxed atomics, for every size and alignment: the DataView get and set, the TypedArray element get and set (`typed_array::get_element`, `set_element`) and CopyDataBlockBytes (`array_buffer::read_raw`, `write_raw`, `copy_bytes`). So two plain accesses never mix sizes; JS allows the tearing of an unordered access. The T0 keyed view case (§6.1) misses for a view on a SAB (the spec path does the byte loop); a byte loop in the hit would make it larger for every view. Non-shared bytes keep the plain path. `Atomics` stay seq-cst of the element width (no ABI change): an `Atomics` access against a plain byte access on the same bytes is still a mixed-size access, route TA-SHARED-MIXED-WIDTH (owner TA-4 TA-ATOMICS, due M2-exit; options: a per-block lock or seqlock for `Atomics`, or a documented platform exception). The element-width text of the bullet above is replaced by this note. Tests: `tests::array_buffer::plain_shared_access_goes_byte_by_byte`, `abi::typed::tests::two_threads_store_two_widths_on_the_same_shared_bytes`, `tests::ic::a_view_load_reads_the_bound_after_resize_grow_and_detach`] [2026-11-11, TA-2 fixer r4 (Codex TA-2 r3; TA-ORCH-13, M6-ORCH-446), as built: the r3 byte loop let a TypedArray element tear (an Int32Array reader that races a writer of 0 and -1 could read 255). IsNoTearConfiguration (ECMA-262 §25.1.3.12) does not allow this: an unordered TypedArray access of an unclamped integer type does not tear. The tear rule now follows the isTypedArray argument, which each call site gives (`array_buffer::IsTypedArray`). (1) No-tear rule: a TypedArray element get and set (`typed_array::get_element`, `set_element`) and GetValueFromBuffer and SetValueInBuffer of a TypedArray (the ABI pair `Cx::get_value_from_buffer`, `set_value_in_buffer`; a byte index that is not a multiple of the size is no element and goes byte by byte) use one Relaxed atomic of the element width, for every element type. The element is aligned by construction; a debug assert checks it. (2) DataView tear rule: a DataView get and set (isTypedArray false) keep the r3 byte loop and can tear. Copies (CopyDataBlockBytes) stay byte by byte. Non-shared bytes, the `Atomics` ABI and the keyed IC miss for a view on a SAB (§6.1; the cold path is the TypedArray path) do not change. (3) Platform exception: two threads can still access the same shared bytes with two widths (two views of different element types, a DataView and a TypedArray, or `Atomics` and a plain access). Aligned loads and stores are single-copy atomic per access on aarch64 and x86-64 (the only ep9 targets). In the Rust abstract model a mixed-size atomic access is UB (Miri would flag it). V8 and SpiderMonkey use the same platform rule. Route TA-SHARED-MIXED-WIDTH (owner TA-4 TA-ATOMICS) picks the final form (a per-block seqlock, or inline-asm accesses outside the Rust model) before any product thread API. For TypedArray elements, this note replaces the byte-loop text of the r3 note. Tests: `tests::array_buffer::plain_shared_access_width_follows_is_typed_array` (the r3 probe test, renamed), `abi::typed::tests::two_threads_see_no_torn_int32_or_int16_element`]
- Waiters: one process list (a `Mutex` map from (block serial, byte index) to a queue of waiters, each with its own `Condvar`). The key is a serial number of the block, not an address. `Atomics.wait` also wakes on an isolate interrupt, so a runner timeout can stop a waiting agent. No thread-local destructor takes the list lock (the Windows exit lesson).
- `Atomics.waitAsync` resolves a promise in the waiting isolate. A notify from another thread posts a host task to that isolate's job queue.
- Agents: `$262.agent.start(src)` starts a thread with its own isolate (same config). `broadcast(sab, n)` hands each agent a new SAB object on the same block (`Arc` clone). The runner joins every agent before the next file and kills one at the file deadline. [2026-11-11, TA-2 fixer r1 (TA-ORCH-11 item 6; harness checker C1), as built: "kills" means terminate. At the end of each run (the file deadline included) the runner terminates every agent isolate (`TerminateHandle`) and waits `agent::STOP_GRACE` (5 s) for its thread; a thread that does not stop is left (Rust cannot kill a thread); the run is an infra error ("timeout: N agent(s) still ran"), never a pass. A worker exit and respawn for a stuck agent is not built (product call settled in TA-ORCH-11). The worker's hard timeout counts the stop grace of each run. `broadcast` as built: `Cx::shared_array_buffer_block` and `Cx::new_shared_array_buffer_on_block` (N1)]

## 6. Inline caches and fuses (H16, H31)

### 6.1 T0 (every preset)

```text
 GetKeyedProperty (interp/ops.rs, unchanged)
   array inline path ── miss ──► keyed_ic::fast_get        (out of line; already the next step today)
                                   view kind? key int32 >= 0?
                                   TYPED_FIXED:    i < +40 ?            ──► load by cell kind
                                   TYPED_VARIABLE: i < bound(buffer) ? ──► load by cell kind
                                   out of bounds:  undefined (no walk)
                                   BigInt views, other keys: cold (generic path)
 SetKeyedProperty ──► keyed_ic::fast_set: view, int32 key, value already a Number (no call),
                      in bounds ──► store by cell kind; else cold
```

- The key half of a StoreKeyed slot keeps its meaning (a shape, or the hole for megamorphic; `crates/ep9_vm/src/keyed_ic.rs:16-22`). A view shape is just another shape. [2026-11-11, TA-2 TA-KEYED as built (246b2d5c): a view store hit leaves the key half alone (BENCH-SET: a view shape in the key half cost +0.72% crypto); `ops_of` gives the view lists only with a view shape as the case key, so no T0 case reaches them until a writer of the element type exists (route TA-KEYED-VIEW-CARRIER, owner TA-3, TA-ORCH-11)]
- A hit never allocates and never calls (`crates/ep9_vm/src/keyed_ic.rs:65`). So a BigInt view load (a new BIGINT cell) and a store that needs ToNumber stay cold.
- The kinds set records bit 17 or 18 for a view receiver on the cold path (the bit rule of H16:234). No new record kind.
- An inline view path in `ops.rs` is a later lever with BENCH data. It must keep the DISASM count and plan rule 72.

### 6.2 Handler kinds

No new handler kind in month 8. The smi kinds have 7, 8 and 15 free, and 7 and 8 are kept for one checked representation (H16:65). The IC_HANDLER kinds are 1-11 (H16:66; `crates/ep9_vm/src/ic.rs:233-278`). `ta.length`, `byteLength` and `byteOffset` are getters on `%TypedArray%.prototype`; the GETTER kind 9 covers them. A later `VIEW_LENGTH` IC_HANDLER kind (12) is a fill when its first reader lands (H16:236), with a fuse on the getter chain. Take it only with profile data.

### 6.3 Op list appends (`ops_of`, H16:144-146)

| Append | Meaning |
|---|---|
| `Class::View(t)` for `GuardKind` (`crates/ep9_vm/src/ic_ops.rs:335-338`) | the cell kind is the view kind of element type `t` |
| `GuardViewIndex(Val, Val)` | the key is an int32 `>= 0` below the view's bound (the bound form follows the elements kind) |
| `LoadView(t)`, `LoadViewOrUndefined(t)` | the element as a Number (Uint32 may give a double; floats give a canonical NaN when boxed); the second gives undefined out of bounds |
| `StoreView(t)` | the store of a Number already checked by `GuardType(Value, Number)`; it misses when the value needs a call |

`keyed()` (`crates/ep9_vm/src/ic_ops.rs:714`) gives these lists when the kinds set has bit 17 or 18. A reader that does not know them sees a miss (`[Generic]`), as H16:236 asks. [2026-11-11, TA-2 TA-KEYED as built (246b2d5c): a view store hit leaves the key half alone (BENCH-SET: a view shape in the key half cost +0.72% crypto); `ops_of` gives the view lists only with a view shape as the case key, so no T0 case reaches them until a writer of the element type exists (route TA-KEYED-VIEW-CARRIER, owner TA-3, TA-ORCH-11)]

### 6.4 Fuses

- "TypedArray species" is static fuse 8 (`crates/ep9_vm/src/fuse.rs:200-212`). It stays unwired in month 8: TypedArraySpeciesCreate does the full `constructor` and `@@species` lookup, as Array species does today (only fuse 6 is wired, `crates/ep9_vm/src/fuse.rs:84`). Its pop site needs a value-store hook, because a store to an existing `constructor` property is not a shape change. That is a later speed item, together with Array species. V8 has the same protector (`src/execution/protectors.h:104-105`).
- No detach fuse. V8 needs `ArrayBufferDetaching` (`src/execution/protectors.h:19-20`) because it does not zero every view; ep9 zeroes them (TA-D5).
- H31:56 row 3 needs a clarify note: the view kinds answer an index through hooks, and `own_index` refuses them (no check result changes). [2026-11-11, TA-1 fixer r2, checker item 4 (M6-ORCH-389, -398): fill, not clarify: the PR changes own_index; the view kinds join the row's rule as a part the plan defers; every existing check keeps its result; G8 0]

### 6.5 FASTDIFF (plan rule 73)

Every fast case lands with cases in `crates/ep9_builtins/tests/fastpath_diff.rs`: in-bounds load and store of each element type; an out-of-bounds load and store; a detached view; a store whose valueOf detaches; Uint8Clamped rounding; a NaN store read back through a Uint8Array; a shrunk resizable buffer; an "auto" view after `grow`; a view as a prototype; a canonical numeric string key. 0 mismatches.

## 7. The modes view (doc 13)

| Tier or mode | A typed element load, month 8 | Later (no contract change needed) |
|---|---|---|
| T0, every preset | §6.1 | an inline `ops.rs` path (lever) |
| JIT preset, T1/T2 (no JIT tier runs JS before the M2 gate, 09:36) | none | lowers the §6.3 list. A failed guard: DEOPT + exit counter (H32:25-31, JIT column) |
| jitless preset, AotBaseline/AotOpt units (13:124 matrix) | none | lowers the same list with the H32 jitless column: SLOWPATH (the generic keyed load) or DEOPT within budget, then PIN. It bakes only the `ep9_vm::layout` facts of §2.2 (OBJECT_LAYOUT 9; a mismatch gives the unit loader's BYTECODE outcome) |
| hybrid | none | DEOPT, then re-tier (H33) |
| facts | kinds-set bits 17/18; `ShapeDesc` kind `typed-array` (8) and elements byte 8 or 9 (value + 1) | the element type in the snapshot: an append with its FACT_SCHEMA bump when the first compiled reader needs it (H16:236) |

- **Why compiled code needs no detach guard.** A detached view has length 0, so the bound guard fails. A TYPED_VARIABLE path reads the buffer's byteLength, which is 0 after detach. So one bound guard covers detach, shrink and out of bounds.
- **M-R9 (13:81).** View shapes, kinds and stores read no `ExecPolicy` and come out the same under every preset and every feedback mode. The rule 40 greps cover `typed_array.rs` and `array_buffer.rs`.
- **F7 (13:608) is not this.** F7 is a typed storage form for plain Arrays in A5 sound regions (13:648: "A5 decides with data"). It takes its own value from the 7 that stay free.
- **SAB in native code.** An aligned plain load in native code is single-copy atomic on both targets, so native code needs no atomic form for element access; `Atomics` calls stay calls in month 8.

## 8. Month-8 work split

### 8.1 Phases (a side branch from the post-merge-6 head, the INTL workflow form)

```text
 TA-0  TA-DECL ─┬─ TA-HEAP                         contracts: ids, layouts, OBJECT_LAYOUT 9, API stubs, heap input
 TA-1  TA-VM ───┘                                  the VM core (Rust-level tests only)
 TA-2  TA-BUFFERS ─ TA-HARNESS ─ TA-KEYED          ArrayBuffer, SAB, DataView; $262 hooks; T0 fast cases
 TA-3  TA-CTORS                                    %TypedArray% and the 12 constructors
 TA-4  TA-PROTO ─ TA-B64 ─ TA-ATOMICS              prototype methods; base64/hex; Atomics, waiters, waitAsync
       ──► the TA merge into the month-8 line (one merge agent; the expectation files by the fixer, plan rule 33)
```

Each phase has its fixer and reviewer as plan §3.9 says. A phase starts on the merge head of the one before it (plan rule 6). A role in a phase uses only APIs that exist on its base: TA-DECL writes the `Cx` method list with `m8-stub` bodies (plan rule 4 form) that return a TypeError, and TA-VM fills them.

### 8.2 Roles and grants (no two roles of a phase on one file)

| Phase | Role (chain) | Grants | Effort (est.) |
|---|---|---|---|
| TA-0 | TA-DECL (VM lead) | `crates/ep9_vm/src/kinds.rs :: appended ids at the end of the const block, the range test, the module doc table rows, table_v1_ranges`; `crates/ep9_vm/src/elements.rs :: kind values 7 and 8, Storage, storage, to_fact, from_fact, storage_form_ok`; `crates/ep9_facts/src/bits.rs :: ElementsKind names 7 and 8`; `crates/ep9_facts/tests/golden/schema.txt :: the ElementsKind and ElementsKindSet name lines` [2026-11-10, TA-0 fixer, TA-ORCH-1: these two items move to TA-3 TA-CTORS; TA-DECL left both files unchanged]; `crates/ep9_vm/src/versions.rs :: OBJECT_LAYOUT_VERSION` and `pins:` the files that pin OBJECT_LAYOUT 8; `crates/ep9_vm/src/layout.rs :: appended OBJECT_LAYOUT 9 exports`; `spikes/spike-c/src/vm_layout.rs :: the PINNED OBJECT_LAYOUT row` (plan rule 12); `crates/ep9_vm/src/abi/slots.rs :: ObjectClass appends`; `crates/ep9_vm/tests/golden/object-classes.txt :: appended lines`; `new:crates/ep9_vm/src/abi/typed.rs` (stubs); `crates/ep9_vm/src/abi/mod.rs :: one mod line`; `crates/ep9_vm/tests/golden/h20-core.txt :: appended lines`; `crates/ep9_vm/tests/golden/public-api.txt :: appended lines` (the M7-P5 TOOLS golden); `new:crates/ep9_math/src/f16.rs`, `crates/ep9_math/src/lib.rs :: one mod line`; `new:crates/ep9_vm/src/tests/layout9.rs`, `crates/ep9_vm/src/tests/mod.rs :: one mod line`; `crates/ep9_vm/src/tests/layout3.rs :: the kind-id list arms`; `crates/ep9_vm/src/tests/elements.rs :: the_fact_kind_map_is_one_to_one want list`; ADR rows: H17 (kind ids fill; new part "view cells"; version row), H18 (values 7 and 8 fill; names), H20 (part 2 fills), H25 (inventory rows) | 2-3 |
| TA-0 | TA-HEAP (GC) | `crates/ep9_heap/src/heap.rs :: the external-bytes input and its GC trigger`; `crates/ep9_heap/src/tests/** :: appended cases`; `docs/plan/03-gc.md :: the S3 row: BYTES blocks are pinned` | 1-2 |
| TA-1 | TA-VM (object) | `new:crates/ep9_vm/src/array_buffer.rs`; `new:crates/ep9_vm/src/typed_array.rs`; `crates/ep9_vm/src/lib.rs :: two mod lines`; `crates/ep9_vm/src/abi/typed.rs`; `crates/ep9_vm/src/runtime.rs :: the BufferTable field`; `crates/ep9_vm/src/exotic.rs :: the view-kind arms`; `crates/ep9_vm/src/property.rs :: own_index and the canonical numeric key arms`; `crates/ep9_vm/src/shape.rs :: may_have_index_keys`; `crates/ep9_vm/src/object.rs :: register_kinds`; `new:crates/ep9_vm/src/tests/typed_array.rs`, `new:crates/ep9_vm/src/tests/array_buffer.rs`, `crates/ep9_vm/src/tests/mod.rs :: two mod lines`; `crates/ep9_vm/src/tests/fuse.rs :: WAYS and the view way` (MC-w26-4); ADR rows: H17 ("view cells" as built; "Index keys on prototypes" fill), H22 (H22-Q8 fill), H25 (rows as built), H31 (row 3 clarify) | 6-9 |
| TA-2 | TA-BUFFERS (SPEC) | `new:crates/ep9_builtins/src/array_buffer.rs` (ArrayBuffer, SharedArrayBuffer); `new:crates/ep9_builtins/src/data_view.rs`; `crates/ep9_builtins/src/lib.rs :: module list lines`; `crates/ep9_vm/src/builtins.rs :: IntrinsicId appends and the lazy group` (the one writer of TA-2); `crates/ep9_vm/tests/golden/vm-builtin-ids.txt :: appended lines`; `crates/ep9_vm/src/tests/builtins.rs :: the exact key-list pin (e2d4f004 form)`; `new:crates/ep9_builtins/tests/golden/ids-array_buffer.txt`, `ids-data_view.txt`; `new:crates/ep9_builtins/tests/array_buffer.rs`; ADR rows: H13 (index ids), H20 (part 2) | 3-4 |
| TA-2 | TA-HARNESS (tools) | `tools/test262/src/exec.rs :: PRELUDE, HOST_FUNCTIONS and the agent hooks`; `new:tools/test262/src/agent.rs`; `tools/test262/src/main.rs :: one mod line`; `tools/test262/src/frontmatter.rs :: the CanBlock flags`; `tools/test262/tests/** :: fixtures`; `after TA-VM: docs/adr/H22-isolates-and-realms.md :: the $262 row` | 2-3 |
| TA-2 | TA-KEYED (VM IC) | `crates/ep9_vm/src/keyed_ic.rs :: the view fast cases`; `crates/ep9_vm/src/ic_ops.rs :: Class, Op appends, keyed`; `crates/ep9_vm/src/facts_export.rs :: the ObjectKind map row`; `crates/ep9_vm/src/tests/ic.rs`, `tests/ic_ops.rs :: appended cases`; `crates/ep9_vm/src/tests/facts_export.rs :: NO_WRITER` (17 -> 16, `:1550`); `crates/ep9_builtins/tests/facts_export.rs :: NO_WRITER_ITEMS` (17 -> 16, `:889`) [2026-11-10, TA-0 fixer, TA-ORCH-1: TA-KEYED keeps both lists at 17; the 17 -> 16 move is TA-3's]; `crates/ep9_builtins/tests/fastpath_diff.rs :: appended view cases`; ADR rows: H16 (op appends fill; the "Declared, no writer" shrink, H16:237) | 3-4 |
| TA-3 | TA-CTORS (SPEC) | `new:crates/ep9_builtins/src/typed_array/mod.rs` (%TypedArray%, the 12 constructors, `from`, `of`, `@@species`, `BYTES_PER_ELEMENT`, TypedArrayCreate and SpeciesCreate); `crates/ep9_builtins/src/lib.rs :: module list lines`; `crates/ep9_vm/src/builtins.rs :: IntrinsicId appends` (the one writer of TA-3); `crates/ep9_vm/tests/golden/vm-builtin-ids.txt :: appended lines`; `crates/ep9_vm/src/tests/builtins.rs :: the exact key-list pin`; `new:crates/ep9_builtins/tests/golden/ids-typed_array.txt`; `new:crates/ep9_builtins/tests/typed_array.rs`; ADR rows: H13, H20 [2026-11-10, TA-0 fixer, TA-ORCH-1: TA-CTORS also takes `crates/ep9_facts/src/bits.rs :: ElementsKind names 7 and 8`, `crates/ep9_facts/tests/golden/schema.txt :: the ElementsKind and ElementsKindSet name lines`, the NO_WRITER and NO_WRITER_ITEMS 17 -> 16 move, and the H18 (names) and H16 ("Declared, no writer" shrink) rows of these items] | 3-5 |
| TA-4 | TA-PROTO (SPEC) | `new:crates/ep9_builtins/src/typed_array/proto.rs` (its own block; its props target `%TypedArray.prototype%`); `crates/ep9_builtins/src/lib.rs :: module list lines`; `crates/ep9_vm/src/natives.rs :: the %ArrayIteratorPrototype%.next view arm` (`:1161`; the one writer of TA-4); `crates/ep9_vm/src/interp/slow/iter.rs :: the view arm` (`:191`); `new:crates/ep9_builtins/tests/typed_array_proto.rs`; `new:crates/ep9_builtins/tests/golden/ids-typed_array_proto.txt` | 5-7 |
| TA-4 | TA-B64 (SPEC) | `new:crates/ep9_builtins/src/typed_array/base64.rs`; `after TA-PROTO: crates/ep9_builtins/src/lib.rs :: one module line`; `new:crates/ep9_builtins/tests/base64.rs`; `new:crates/ep9_builtins/tests/golden/ids-base64.txt` | 1-2 |
| TA-4 | TA-ATOMICS (SPEC) | `new:crates/ep9_builtins/src/atomics.rs`; `after TA-B64: crates/ep9_builtins/src/lib.rs :: one module line`; `crates/ep9_vm/src/builtins.rs :: the %Atomics% IntrinsicId` (the one writer of TA-4); `crates/ep9_vm/tests/golden/vm-builtin-ids.txt :: appended lines`; `crates/ep9_vm/src/tests/builtins.rs :: the exact key-list pin`; `crates/ep9_vm/src/array_buffer.rs :: the waiter list`; `crates/ep9_vm/src/jobs.rs :: the host task for waitAsync`; `new:crates/ep9_builtins/tests/atomics.rs`; `new:crates/ep9_builtins/tests/golden/ids-atomics.txt` | 4-6 |

Total 30-45 agent-days (est.). Critical path TA-DECL -> TA-VM -> TA-BUFFERS -> TA-CTORS -> TA-PROTO: 19-28 agent-days (est.).

### 8.3 One-writer files and versions (plan §11.2, §11.3)

| File or version | TA-0 | TA-1 | TA-2 | TA-3 | TA-4 |
|---|---|---|---|---|---|
| `builtins.rs :: IntrinsicId` and `vm-builtin-ids.txt` | none | none | TA-BUFFERS | TA-CTORS | TA-ATOMICS |
| `natives.rs` | none | none | none | none | TA-PROTO |
| `Cargo.toml` tables and `Cargo.lock` | none | none | none | none | none (no new crate; `Reservation` and `std` atomics are enough) |
| `runtime-table.txt` | none | none | none | none | none |
| `crates/ep9_builtins/src/lib.rs :: module list` | none | none | TA-BUFFERS | TA-CTORS | TA-PROTO, then TA-B64, then TA-ATOMICS (`after`) |
| OBJECT_LAYOUT 8 -> 9 | TA-DECL | - | - | - | - |
| ABI, IC_LAYOUT, DEOPT_META, VM_LAYOUT, FACT_SCHEMA, unit container | - | - | - | - | - |
| `expectations/test262.*.txt` | the phase fixer, each phase (plan rule 33) | | | | |

- BuiltinId blocks: each new module takes the next free block at its base (7-bit block, 9-bit index; `crates/ep9_vm/src/builtins.rs:17`). 6 modules -> 6 blocks (est. 140 ids).
- IntrinsicIds (est. 33): `%ArrayBuffer%`, `%SharedArrayBuffer%`, `%DataView%` and their prototypes (TA-2); `%TypedArray%`, its prototype, the 12 constructors and the 12 prototypes (TA-3); `%Atomics%` (TA-4). They follow the INTL wave-1 ids.
- Main-line month-8 roles get no grant on these files and parts while the TA branch holds them (check rule 18 across the side branch, as INTL had). The main line keeps `interp/ops.rs`, `ic.rs` and the dispatch core.

### 8.4 ADR changes and their kind (no G8 count)

| ADR part | Change | Kind (AR:74-77) | Frozen check result |
|---|---|---|---|
| H17 "kind ids (append-only)" (H17:177) | 13 ids | fill | same (count pins move as part of the fill, H17:269) |
| H17 new part "view cells" | the layouts of §2.2 | rewrite, pre-freeze churn (H17:270 form) | no Frozen part changes |
| H17 OBJECT_LAYOUT row | 8 -> 9 | rewrite (version) on the open parts (H17:271 form) | layout core text the same [2026-11-10, TA-0 fixer, clarify] The OBJECT_LAYOUT number in the H17 layout-core What and Check cells moves 8 -> 9 (H17 clarify row 2026-11-10, item (2)); no layout-core fact changes. |
| H17 "Index keys on prototypes" | the view way (§4.2) | fill | the 6 ways keep their result; the way list grows (MC-w26-4) |
| H18 "elements kind values (append-only)" (H18:85) | 7, 8 | fill | §3.2 table |
| H18 "FactSnapshot kind list" (H18:88) | names 7, 8 | fill (H18:88: "a new name is part of an append's fill") [2026-11-10, TA-0 fixer, TA-ORCH-1: not in TA-0; in TA-3, with a writer, because H16:237 lets a no-writer item come only with a FACT_SCHEMA bump] | `schema.txt` name lines outside the fingerprint |
| H16 "IC lists (append-only)" (H16:236) | §6.3 ops | fill (first reader: the ops tests) | an unknown op is a miss |
| H16 "declared fields (shrink-only)" (H16:237) | `ObjectKind::TypedArray` leaves the list | the part's own rule | the two lists shrink by 1 [2026-11-10, TA-0 fixer, TA-ORCH-1: in TA-3, not TA-2] |
| H20 part 2 (H20:89) | ObjectClass, IntrinsicId, BuiltinId, `Cx` methods | fill | h20-core existing lines unchanged (H20ADD) |
| H25 inventory | 2 rows | fill (H25:61) | extref-lint passes |
| H13 "index ids (append-only)" (H13:93) | the new ids | fill | - |
| H22-Q8 (H22:92), the `$262` row (H22:46) | SAB blocks across isolates; the hooks | fill; clarify | - |
| H31 fast-path row 3 (H31:56) | the `own_index` refusal | clarify [2026-11-11, TA-1 fixer r2, checker item 4 (M6-ORCH-389, -398): fill, not clarify: the PR changes own_index; the view kinds join the row's rule as a part the plan defers; every existing check keeps its result; G8 0] | `is_routed` stays one range test |

G8 count of the plan: 0. If a role finds that one of these changes flips a Frozen check, it stops that item and reports it (plan rule 69).

## 9. Test plan

### 9.1 MC-w26-4 (route row R:100)

| Test | What it proves | Fails today because |
|---|---|---|
| `tests::fuse::each_way_an_index_key_reaches_a_prototype_pops_the_fuse_in_every_realm` with a 7th way (`crates/ep9_vm/src/tests/fuse.rs:779`, `WAYS` 6 -> 7): `way_view_prototype` makes a length-0 Int8Array a prototype | the fuse pops at length 0, in realm 1 and in realm 2, plain and under GC stress | no view kind exists; after TA-1 without the `may_have_index_keys` arm it fails (the arm must make it pass) |
| JS: `o = Object.create(new Int8Array(0)); o[0] = 1` gives no own "0"; `o[0]` is undefined | the store path honors §10.4.5.5 when the fuse popped | node result in §4.2 |
| JS: a holey Array whose prototype chain gets a view later: `a[1]` store and load | the hole path walks the chain after the pop | - |
| JS: a length-4 view prototype, index 9 store; `Object.prototype[9]` is not seen through the view | OOB at any length | - |

### 9.2 The rest

- UNIT(ep9_vm): each element type round trip; NaN patterns; Uint32 and Float16 edges; detach zeroes every view (2 views, one dead before the GC); a dead buffer frees its table entry after a minor and a major GC; `transfer` moves a BYTES cell with no copy; `resize` keeps the base address; each test also under GC stress 1.
- UNIT(ep9_builtins): the JS-level tables of §4.1 with the node results.
- FASTDIFF: §6.5, 0 mismatches.
- T262-SUB on the Mac (per phase): `built-ins/ArrayBuffer`, `SharedArrayBuffer`, `DataView` (TA-2); `TypedArrayConstructors` (TA-3); `TypedArray`, `Uint8Array`, `Atomics` (TA-4). The full run is GitHub CI only.
- BENCH-SET per role (plan rule 13): the `own_index` compare and the `fast_get` arm are on paths that the 7 kernels use; no kernel and no t0-* row may go above +0.5%.
- GCLINT and plan rule 8 review of `array_buffer.rs` (the weak source and the table).
- The agent tests have a deadline and a kill path; a waiter that never gets a notify ends at the deadline.

## 10. Comparison with other engines

| Item | V8 | JSC | SpiderMonkey | ep9 (this design) |
|---|---|---|---|---|
| element type | one elements kind per type, 12 + 12 RAB/GSAB (`src/objects/elements-kind.h:19-48`) | `JSType` per type (`runtime/JSType.h:100-112`); DFG `Array::Type` (`dfg/DFGArrayMode.h:53-78`) | a `JSClass` per (form, type) (`vm/TypedArrayObject.h:53-63`) | cell kind per type (TA-D1) |
| fixed vs resizable | separate elements kinds; bits `is_length_tracking`, `is_backed_by_rab` (`src/objects/js-array-buffer.h:460-461`) | the view mode (`runtime/JSArrayBufferView.h:59-104`); DFG speculates on it (`dfg/DFGArrayMode.h:135`) | `FixedLengthTypedArrayObject` / `ResizableTypedArrayObject` (`vm/TypedArrayObject.h:147`, `:189`); CacheIR `ArrayBufferViewKind` (`jit/CacheIR.h:597-601`) | elements kind 7 / 8 (TA-D2) |
| small buffers | on heap up to 64 bytes (`BUILD.gn:333-334`) | on heap up to 1,000 elements (`runtime/JSArrayBufferView.h:191`) | inline data, lazy buffer object (`vm/TypedArrayObject.h:150-153`) | every fixed buffer on heap (BYTES; non-moving heap); a lazy buffer object is a later lever |
| detach | one view tracked, else a protector pops (`src/objects/js-array-buffer.cc:197-243`) | zero every view (`runtime/ArrayBuffer.cpp:532-540`, `runtime/JSArrayBufferView.cpp:260-269`) + a watchpoint (`runtime/ArrayBuffer.h:362`) | zero every view: first view slot + `InnerViewTable` (`vm/ArrayBufferObject.cpp:1299-1325`; `vm/ArrayBufferObject.h:890`) | zero every view through a weak list (TA-D5); no fuse |
| view slots | buffer, length, byteOffset, data pointer | `m_vector`, `m_length`, `m_byteOffset` | `BUFFER_SLOT`, `LENGTH_SLOT`, `BYTEOFFSET_SLOT`, `DATA_SLOT` (`vm/ArrayBufferViewObject.h:34-53`) | buffer, data, length, byteOffset (§2.2) |
| IC ops | keyed element handlers by elements kind | DFG array modes | `LoadTypedArrayElementResult`, `StoreTypedArrayElement`, `GuardIsResizableTypedArray` (`jit/CacheIROps.yaml:2399`, `:1891`, `:519`) | §6.3 appends |
| species | `TypedArraySpeciesProtector` (`src/execution/protectors.h:104-105`) | a watchpoint set per type (`runtime/JSGlobalObject.h:629`) | `OptimizeTypedArraySpeciesFuse` (`vm/RealmFuses.h:218`) | static fuse 8, unwired in month 8 (full lookup) |
| waiters | `FutexEmulation` (`src/execution/futex-emulation.h:212`) | `WaiterListManager` (`runtime/WaiterListManager.h:205`) | `FutexThread` (`builtin/AtomicsObject.h:63`) | one process list (§5.6) |
| QuickJS | - | - | - | a list of views per buffer, `track_rab` (`quickjs.c:782-791`); detach updates them (`quickjs.c:60927-60939`) |

## 11. Decisions the orchestrator makes before TA-0

1. The H18 value row (13:550): Proposal A (values 7 and 8; 7 remain) or fallback B (§3.4).
2. The TA side branch name, its merge gate and the phase list of §8.1 (or a compressed form: TA-0 and TA-1 can join if one role does both, est. 8-11 agent-days).
3. The OBJECT_LAYOUT 9 writer (TA-DECL) in the month-8 version table.
4. The main-line month-8 roles stay off the files of §8.2 while the branch is open.
5. The size caps of §2.3 (est. values), or other values with data.
