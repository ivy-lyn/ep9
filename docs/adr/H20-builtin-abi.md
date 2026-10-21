# H20: Builtin ABI

| Field | Value |
|---|---|
| Status | Draft (2026-10-21; ABI v1 draft, M0 wk4 P1; tables, ids, crate layout and the P3 surface, M0 wk4 P2) |
| Fix by | M0 |
| Freeze point | M0 exit |
| Origin | v1#16; plan status U |
| Crates | `ep9_vm` (`abi`, host calls in `call.rs`, ObjectOps internals in `property.rs`, the table types and install in `builtins.rs`, the VM table in `natives.rs`); `ep9_builtins` (`forbid(unsafe_code)`); embedders `ep9_cli`, `tools/test262` |
| Owner (plan role) | VM lead (builtins) |
| Validated by | `ep9_vm` tests `tests::abi`, `tests::builtins`, `tests/builtin_ids.rs`; `ep9_builtins` tests `tests/table.rs` and the `TEMPLATES.md` doctests; test262 exec and its GC-stress run; `cargo xtask gc-lint` (P2) |
| Plan sources | [02 §6 H20](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [09 §8 row 4](../plan/09-milestones.md), M0 wk4 build plan §4 P1-ABI, §5 P2-BTAB and P2-LINT |

## Context

- The most-copied code shape: thousands of builtins. A change after the freeze touches all of them [R07: Brimstone's handle refactor].
- Depends on: H10 (CSS keeps Rust locals alive), H14 (frame layout: `Args` reads the host frame), H06 (`Value` wraps `NonZeroU64`), H19 (atoms are the property keys), H22 (realms, R5). Depended on by: the builtins (P2, P3), H13 (BuiltinId index list), H35 (unit link cells), the JIT and AOT host-call stub.

## Decision

Plan row: `fn(cx, this, args, new_target) -> JsResult<Value>`. `Args` is a frame view; `Throw` is a ZST; no `&mut` to a heap object across a JS call. AOT code reaches builtins through the isolate table or the link table (02 §6). The v1 draft keeps this shape with `Local` for `Value` (a rewrite of the row's result type before the freeze; P4-DOCS updates 02):

```rust
pub type HostFn = for<'s> fn(&mut Cx<'s>, Local<'s>, Args<'s>, Local<'s>) -> JsResult<Local<'s>>;
//                           cx           this       args (= fp)  new_target
```

| Type | What it is | Where |
|---|---|---|
| `Cx<'s>` | the context: ObjectOps, conversions, calls, `throw_*`; exposes no `&Runtime` | `crates/ep9_vm/src/abi/mod.rs:477` |
| `Local<'s, T>` | the value itself, `repr(transparent)` over `Value`; `Copy`, `!Send`, `!Sync`; markers `JsValue` (default), `JsObject`, `JsFunction`, `JsArray`, `JsString`, `FlatStr`, `JsSymbol`, `JsKey`; checked downcasts, `From` upcasts | `crates/ep9_vm/src/abi/local.rs:84` |
| `Args<'s>` | one word, the host frame pointer: `len`, `get`, `this`, `new_target`, `callee`, `rest(i)` | `crates/ep9_vm/src/abi/mod.rs:69` |
| `PropertyDescriptor<'s>` | §6.2.6, every field optional | `crates/ep9_vm/src/abi/mod.rs:197` |
| `ScratchList<'s>` | LIFO precise list over `ep9_heap::ScratchRoots` | `crates/ep9_vm/src/abi/mod.rs:346` |
| `ArgsBuilder<'s>` | call arguments on the VM stack above the frame top, under the scratch top (no heap list) | `crates/ep9_vm/src/abi/mod.rs:377` |

**ObjectOps v0** (`Cx` methods, spec names): `get`, `get_v`, `set`, `has_property`, `has_own_property`, `get_own_property`, `define_own_property` (ValidateAndApplyPropertyDescriptor), `define_property_or_throw`, `create_data_property(_or_throw)`, `delete`, `delete_property_or_throw`, `own_property_keys` (a JS array: indices ascending, strings in creation order, symbols), `get/set_prototype_of`, `is_extensible`, `prevent_extensions`, `is_callable`, `is_constructor`, `is_array`. Conversions: `to_boolean`, `to_number`, `to_integer_or_infinity`, `to_int32`, `to_uint32`, `to_length`, `to_string`, `to_property_key`, `to_object`, `to_primitive`, `same_value`, `is_strictly_equal`. Spec helpers: `to/from_property_descriptor`, `create_list_from_array_like` (into an `ArgsBuilder`), `get_prototype_from_constructor`, `ordinary_create_from_constructor`. Creation: `new_object(_with_proto)`, `new_array`, `create_array_from_list`, `new_string(_utf16)`, `new_wrapper`, `new_symbol`, `intrinsic(IntrinsicId)`. P3 surface (P2-BTAB): `symbol_for`, `key_for`, `symbol_description`, `descriptive_string`, `this_primitive_value(v, PrimitiveType)`, `set_function_name`, `set_function_length`, `new_bound_function(target, this, args.rest(i))`, `has_intrinsic`. Calls: `call`, `call_args(f, this, args.rest(i))`, `call_built`, `construct(f, args, new_target)` with any constructor as new.target, `construct_args`, `construct_built`. v0 limits: an accessor descriptor throws "not implemented" (accessors are P3); array `length` stays writable.

**JIT and AOT** never call a `HostFn` (Rust ABI). They call one `extern "C"` host-call stub `(cx, fp) -> bits` that pushes the host frame and runs `call::call_host`; the builtin is named by its FunctionInfo or its `BuiltinId` (block 7 bits + index 9 bits) through the isolate table or a unit link cell (H35).

### Builtin tables (M0 wk4 P2)

```text
IsolateConfig { builtins: Some(&ep9_builtins::TABLE), .. }
Isolate::new
  builtins::check      both tables pass check_tables (cached per table)  -> IsolateError::BuiltinTable
  Runtime::new
  builtins::prepare    one dense FunctionInfo array + one root range of function slots per module
  realm::init          atoms; core intrinsics: %Object.prototype%, %Function.prototype% (VM builtin 0.0),
                       Array/String/Number/Boolean/Iterator/ArrayIterator/Error prototypes, the global
  builtins::install    VM table (block 0), then the embedder table, modules in order:
                       pass 1 every function object; pass 2 the intrinsics; pass 3 the properties
```

| Type | What it is | Where |
|---|---|---|
| `BuiltinId(u16)` | block (7 bits, < 128) + index (9 bits, < 512): definition `i` of the module with block `b`. Blocks are fixed per module; ids are append-only | `crates/ep9_vm/src/builtins.rs:51` |
| `IntrinsicId` | 32 append-only ids: the core prototypes 0-16 (made by realm init; `SymbolPrototype` by `symbol::install`), then the constructors and `%Math%` 17-31 (made by tables). Each maps to a runtime root | `crates/ep9_vm/src/builtins.rs:95` |
| `BuiltinDef` | `{name, length, kind: Method \| Getter \| Setter \| Constructor, f: HostFn}`; a getter is named "get x" | `crates/ep9_vm/src/builtins.rs:300` |
| `PropDef` | `{key, value, attrs}`. Key: `Atom` (the `atoms!` list), `Str` (interned at init), `Symbol` (well-known). Value: `Builtin(i)`, `Intrinsic`, `Const`, `Symbol`, `Alias(key)` (the value of a key defined earlier on the same object: one function object under two names), `Accessor {get, set}` | `crates/ep9_vm/src/builtins.rs:513` |
| `BuiltinModule`, `IntrinsicDef`, `ObjectProps` | a module: name, block, `defs`, the intrinsics it makes (`Builtin(i)` or a new ordinary object), the properties per target intrinsic | `crates/ep9_vm/src/builtins.rs:593` |
| `BuiltinTable` | a name and its modules; `check_tables` checks a list of tables | `crates/ep9_vm/src/builtins.rs:672`, `:715` |

`check_tables` rejects: a block ≥ 128 or used twice; more than 512 definitions; an intrinsic made twice, or a core one made other than as a VM builtin; a property on, or a reference to, an intrinsic nothing makes; a definition index out of range or of the wrong kind (`Builtin` needs a Method or Constructor, `Accessor` a Getter/Setter); an `Alias` of a key not defined earlier on the same object; a key defined twice on one object. Install errors (a well-known symbol or an accessor that does not exist yet) fail `Isolate::new` with a message naming the module and key.

**One function object per builtin.** `builtins::function_object` makes it on first use and keeps it in the module's root slot, so an `Alias`, an intrinsic and a property name one object. The FunctionInfos sit in one boxed slice per module (no Box per function); `rt.infos` keeps only embedder host functions.

**Ids are golden-checked.** `crates/ep9_vm/tests/golden/vm-builtin-ids.txt` (the VM module plus the `IntrinsicId` list) and `crates/ep9_builtins/tests/golden/ids-<module>.txt` hold one line per definition; a reorder fails them, and `EP9_BLESS=1` rewrites them after a review that sees only appended lines.

**The VM table** (`crates/ep9_vm/src/natives.rs:289`, block 0): `%Function.prototype%` (a function that returns undefined, §20.2.3), `Function` (throws "not implemented in ep9 v0: Function constructor"), `Object` (new.target, then `to_object`, §20.1.1.1), `Array`, the 7 error constructors, `Object.prototype.toString`/`valueOf`, `Array.prototype.join`/`toString`/`push`, `%ArrayIteratorPrototype%.next`, `Error.prototype.toString`, and the globals `globalThis`, `undefined`, `NaN`, `Infinity`.

**Crate layout.** `ep9_builtins` (engine dependencies ep9_vm and ep9_math only, `#![forbid(unsafe_code)]`) has one file per module, each with `BLOCK`, `DEFS`, `INTRINSICS`, `PROPS` and `MODULE`: object 1, function 2, array 3, number 4, boolean 5, string 6, symbol 7, math 8, global 9. `TABLE` lists them in that order, which is the property order. Its `clippy.toml` repeats the root bans (clippy reads only the nearest file) and adds `disallowed-types`: `ep9_value::Value`, `Unpacked`, `Word0` and the ep9_heap cell types (re-exports such as `ep9_vm::Value` resolve to them). `TEMPLATES.md` is the writing guide, and its code blocks are doctests. `ep9_cli` and `tools/test262` pass `TABLE`; test262 `exec` exits 2 when an isolate has no table.

**Embedders get `Local` too** (H20-Q4): `Isolate::with_cx(|cx| ...)` runs a closure with a `Cx`; the `for<'s>` bound keeps every `Local` inside it (a `compile_fail` doctest proves the escape does not compile).

## Rules

| Rule | Check | From |
|---|---|---|
| R1. A `Local` in a Rust local stays live across every call (CSS) | GC-stress tests: `tests::abi`, the test262 GC-stress run | M0 wk4 |
| R2. No `Local`, `Args`, `Rest`, `PropertyDescriptor`, `ArgsBuilder` or `ScratchList` in Rust heap memory, a static or a thread_local | `'s` + `!Send` (compiler); `cargo xtask gc-lint` (P2-LINT); debug on-stack check in `Local::bits` (`crates/ep9_vm/src/abi/local.rs:117`, P2-LINT) | M0 wk4 |
| R3. Every `&mut Cx` method may allocate and run JS; a view into heap memory borrows `&Cx`, so none lives across one | borrow checker (`code_unit_at`, `string::view`) | M0 wk4 |
| R4. `Err(Throw)` only with a pending exception, `Ok` only without one; a callee's `Err` passes on unchanged | debug asserts in `call_host` (`crates/ep9_vm/src/call.rs:191`); tests `*_panics_in_debug` | M0 wk4 |
| R5. A builtin creates objects in its callee's realm | v0: one realm; `Args::callee` exists for N realms (H20-Q2) | M1 |
| R6. [[GetOwnProperty]] reports virtual properties with spec attributes: array `length` {w}, elements {w,e,c}; function `length`/`name` {c}, `prototype` {w}; String wrapper `length` and indices | `crates/ep9_vm/src/property.rs:572`; test `virtual_property_descriptors` | M0 wk4 |
| `Local`, `Args`, `Cx`, `JsResult<Local>` are 8 bytes | `const` asserts (`crates/ep9_vm/src/abi/mod.rs:57`); test `sizes_are_one_word` | M0 wk4 |
| `ScratchList` and `ArgsBuilder` nest LIFO; out of order panics | `ScratchRoots` depth check; `ArgsBuilder` scratch-top check (`crates/ep9_vm/src/abi/mod.rs:455`) | M0 wk4 |
| A Rust→JS entry or host call starts above the scratch area and restores the caller's scratch top on return; bytecode never runs with one | `call::top_of`, `restore_scratch` (`crates/ep9_vm/src/call.rs:110`); test `args_builder_values_survive_gc_stress_before_the_call` (fails with the scratch scan off) | M0 wk4 |
| Builtin crates never name `ep9_value::Value` or a heap cell type, and have no `unsafe` | `crates/ep9_builtins/clippy.toml` `disallowed-types` (test `clippy_toml_repeats_the_root_bans`); `#![forbid(unsafe_code)]` | M0 wk4 |
| Builtin ids and `IntrinsicId`s are append-only | the id goldens; tests `swapping_two_entries_fails_the_golden` (ep9_vm) and `swapping_two_entries_fails_a_golden` (ep9_builtins) | M0 wk4 |
| Every table passes `check_tables` before an isolate uses it; one function object per builtin; two isolates give the same property order | `IsolateError::BuiltinTable`; tests `table_checks_reject_each_broken_rule`, `a_prop_def_alias_gives_the_same_function_object`, `two_isolates_have_the_same_intrinsic_property_order` (both crates) | M0 wk4 |

## Consequences

- **Gains.** A `Local` costs nothing (no handle scope); a host call passes 4 integer words (cx, this, args, new_target), which fits the 4 Win64 argument registers, and returns in one register. Builtins reach the VM only through `Cx`, so `forbid(unsafe_code)` holds in `ep9_builtins`. No `Vec<Local>` is ever needed: lists are a `ScratchList`, a JS array or an `ArgsBuilder`.
- **Costs.** Every builtin spells `<'s>`. R2 is a lint plus a debug check, not a type error. `call`/`apply` copy the arguments once (H20-Q3). The LIFO checks run at run time.
- **Costs of the tables.** A block holds at most 512 builtins, and 128 blocks exist; an id never moves, so a removed builtin keeps its slot. A table error shows at `Isolate::new`, not at compile time (the checks are cached per table).
- **Follow-ups.** P2-LINT: gc-lint and the debug check. P3: accessors in `define_own_property` (then `PropValue::Accessor` installs), the builtins of `TEMPLATES.md`. The merge with P2-SYM fills the symbol entry points and `%Symbol.prototype%`. H22: realms for R5. Freeze at M0 exit.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R07] | Brimstone did a whole-runtime refactor to GC-safe handles at month 6; Nova's code is "filled to the brim" with bind/unbind calls, and its author regrets it | [07-rust-engine-landscape.md](../research/07-rust-engine-landscape.md) |
| [R05] | A direct value in a native frame is a root only under CSS: V8 says "do not use direct handles without conservative stack scanning" | [05-gc-design.md](../research/05-gc-design.md) |
| [M] | Sizes, the throw protocol, LIFO panics and GC-stress survival of builder values: `tests::abi` (15 tests); test262: 0 new failures, language/ 8,683 | this ADR, 2026-10-21 |
| [M2] | Tables: `tests::builtins` (19 tests), `tests/builtin_ids.rs` (3), `ep9_builtins` `tests/table.rs` (8) and 6 `TEMPLATES.md` doctests; the disallowed-types bans fire on `ep9_vm::Value` in `src/` and on `Slot`, `CellPtr`, `Heap` in a test (probes, reverted) | M0 wk4 P2-BTAB, 2026-10-21 |

## Validation

- `cargo test -p ep9_vm` (`tests::abi`), test262 exec and its GC-stress run on 4 targets; from P2, `cargo xtask gc-lint` in CI.
- The freeze review at M0 exit checks every row above against the built builtins (P3 templates).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H20-Q1 | Where does `new.target` live? (= H14-Q1.) v0: the feedback slot of a `[[Construct]]` frame; `Args::new_target` reads it only when the construct bit is set | M1 | A register filled at entry (H14-Q1) |
| H20-Q2 | The realm of a builtin with N realms (with H22-Q3, Q7) | M1 (`$262.createRealm`) | The callee's realm, read through `Args::callee` |
| H20-Q3 | `call`/`apply` without the copy | M1 | One copy now (`call_args`, `ArgsBuilder` into the callee frame); a tail-call host path in M1, as V8's Function.prototype.call shifts the arguments and tail-calls |
| H20-Q4 | The embedder API (`Isolate`) still hands out raw, unrooted `Value`s: `let v = iso.run_script(..)?; iso.collect_garbage(); iso.display(v)` reads freed memory in safe code. Partly settled (wk4 P2): `Isolate::with_cx` is the `Cx` scope, and a `compile_fail` doctest shows a `Local` cannot leave it. `run_script`, `call`, `global` and `display` still take or return `Value`, because the frozen `crates/ep9_vm/src/tests/mod.rs` and every embedder use them; until then an embedder must not hold a `Value` across a collection | M0 exit | Move the embedders to `with_cx`, then make those four return `Local` inside a scope or drop them |

## Reopens if

- A GC-stress failure is traced to a `Local` in Rust heap memory: add a type-aware dylint lint (ABI-5 e).
- The JIT/AOT host-call stub needs more than 4 integer arguments on Win64.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft: signature, `Args`, `Local`, R1-R6, ObjectOps v0, lint plan, host-call stub, Q1-Q4 (M0 wk4 plan §4 P1-ABI; decisions W4-ABI-*) | P1-ABI agent |
| 2026-10-21 | draft | Builtin tables: `BuiltinId`, `IntrinsicId`, `BuiltinDef`, `PropDef`, `check_tables`, install order, id goldens, the VM table, the `ep9_builtins` layout and bans, the P3 surface, `Isolate::with_cx` (H20-Q4 partly settled) (M0 wk4 plan §5 P2-BTAB; decisions W4-BTAB-*) | P2-BTAB agent |
