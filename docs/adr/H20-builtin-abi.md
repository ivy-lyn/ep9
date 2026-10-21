# H20: Builtin ABI

| Field | Value |
|---|---|
| Status | Draft (2026-10-21; ABI v1 draft, M0 wk4 P1). The tables, ids and crate layout come in P2 |
| Fix by | M0 |
| Freeze point | M0 exit |
| Origin | v1#16; plan status U |
| Crates | `ep9_vm` (`abi`, host calls in `call.rs`, ObjectOps internals in `property.rs`, `natives.rs`); `ep9_builtins` (P2, `forbid(unsafe_code)`); embedders `ep9_cli`, `tools/test262` |
| Owner (plan role) | VM lead (builtins) |
| Validated by | `ep9_vm` tests `tests::abi`; test262 exec and its GC-stress run; `cargo xtask gc-lint` (P2) |
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
| `Cx<'s>` | the context: ObjectOps, conversions, calls, `throw_*`; exposes no `&Runtime` | `crates/ep9_vm/src/abi/mod.rs:424` |
| `Local<'s, T>` | the value itself, `repr(transparent)` over `Value`; `Copy`, `!Send`, `!Sync`; markers `JsValue` (default), `JsObject`, `JsFunction`, `JsArray`, `JsString`, `FlatStr`, `JsSymbol`, `JsKey`; checked downcasts, `From` upcasts | `crates/ep9_vm/src/abi/local.rs:84` |
| `Args<'s>` | one word, the host frame pointer: `len`, `get`, `this`, `new_target`, `callee`, `rest(i)` | `crates/ep9_vm/src/abi/mod.rs:64` |
| `PropertyDescriptor<'s>` | §6.2.6, every field optional | `crates/ep9_vm/src/abi/mod.rs:192` |
| `ScratchList<'s>` | LIFO precise list over `ep9_heap::ScratchRoots` | `crates/ep9_vm/src/abi/mod.rs:293` |
| `ArgsBuilder<'s>` | call arguments on the VM stack above the frame top, under the scratch top (no heap list) | `crates/ep9_vm/src/abi/mod.rs:324` |

**ObjectOps v0** (`Cx` methods, spec names): `get`, `get_v`, `set`, `has_property`, `has_own_property`, `get_own_property`, `define_own_property` (ValidateAndApplyPropertyDescriptor), `define_property_or_throw`, `create_data_property(_or_throw)`, `delete`, `delete_property_or_throw`, `own_property_keys` (a JS array: indices ascending, strings in creation order, symbols), `get/set_prototype_of`, `is_extensible`, `prevent_extensions`, `is_callable`, `is_constructor`, `is_array`. Conversions: `to_boolean`, `to_number`, `to_integer_or_infinity`, `to_int32`, `to_uint32`, `to_length`, `to_string`, `to_property_key`, `to_object`, `to_primitive`, `same_value`, `is_strictly_equal`. Spec helpers: `to/from_property_descriptor`, `create_list_from_array_like` (into an `ArgsBuilder`), `get_prototype_from_constructor`, `ordinary_create_from_constructor`. Creation: `new_object(_with_proto)`, `new_array`, `create_array_from_list`, `new_string(_utf16)`, `new_wrapper`, `new_symbol` (P2), `intrinsic(IntrinsicId)`. Calls: `call`, `call_args(f, this, args.rest(i))`, `call_built`, `construct(f, args, new_target)` with any constructor as new.target, `construct_args`, `construct_built`. v0 limits: an accessor descriptor throws "not implemented" (accessors are P3); array `length` stays writable.

**JIT and AOT** never call a `HostFn` (Rust ABI). They call one `extern "C"` host-call stub `(cx, fp) -> bits` that pushes the host frame and runs `call::call_host`; the builtin is named by its FunctionInfo or its `BuiltinId` (block 7 bits + index 9 bits, P2) through the isolate table or a unit link cell (H35).

## Rules

| Rule | Check | From |
|---|---|---|
| R1. A `Local` in a Rust local stays live across every call (CSS) | GC-stress tests: `tests::abi`, the test262 GC-stress run | M0 wk4 |
| R2. No `Local`, `Args`, `Rest`, `PropertyDescriptor`, `ArgsBuilder` or `ScratchList` in Rust heap memory, a static or a thread_local | `'s` + `!Send` (compiler); `cargo xtask gc-lint` (P2-LINT); debug on-stack check in `Local::bits` (`crates/ep9_vm/src/abi/local.rs:117`, P2-LINT) | M0 wk4 |
| R3. Every `&mut Cx` method may allocate and run JS; a view into heap memory borrows `&Cx`, so none lives across one | borrow checker (`code_unit_at`, `string::view`) | M0 wk4 |
| R4. `Err(Throw)` only with a pending exception, `Ok` only without one; a callee's `Err` passes on unchanged | debug asserts in `call_host` (`crates/ep9_vm/src/call.rs:191`); tests `*_panics_in_debug` | M0 wk4 |
| R5. A builtin creates objects in its callee's realm | v0: one realm; `Args::callee` exists for N realms (H20-Q2) | M1 |
| R6. [[GetOwnProperty]] reports virtual properties with spec attributes: array `length` {w}, elements {w,e,c}; function `length`/`name` {c}, `prototype` {w}; String wrapper `length` and indices | `crates/ep9_vm/src/property.rs:572`; test `virtual_property_descriptors` | M0 wk4 |
| `Local`, `Args`, `Cx`, `JsResult<Local>` are 8 bytes | `const` asserts (`crates/ep9_vm/src/abi/mod.rs:52`); test `sizes_are_one_word` | M0 wk4 |
| `ScratchList` and `ArgsBuilder` nest LIFO; out of order panics | `ScratchRoots` depth check; `ArgsBuilder` scratch-top check (`crates/ep9_vm/src/abi/mod.rs:402`) | M0 wk4 |
| A Rust→JS entry or host call starts above the scratch area and restores the caller's scratch top on return; bytecode never runs with one | `call::top_of`, `restore_scratch` (`crates/ep9_vm/src/call.rs:110`); test `args_builder_values_survive_gc_stress_before_the_call` (fails with the scratch scan off) | M0 wk4 |
| Builtin crates never name `ep9_value::Value` or a heap cell type | `ep9_builtins` clippy.toml `disallowed-types` (P2-BTAB) | M0 wk4 |

## Consequences

- **Gains.** A `Local` costs nothing (no handle scope); a host call passes 4 integer words (cx, this, args, new_target), which fits the 4 Win64 argument registers, and returns in one register. Builtins reach the VM only through `Cx`, so `forbid(unsafe_code)` holds in `ep9_builtins`. No `Vec<Local>` is ever needed: lists are a `ScratchList`, a JS array or an `ArgsBuilder`.
- **Costs.** Every builtin spells `<'s>`. R2 is a lint plus a debug check, not a type error. `call`/`apply` copy the arguments once (H20-Q3). The LIFO checks run at run time.
- **Follow-ups.** P2-BTAB: tables, `BuiltinId`, `IntrinsicId`, `PropDef`, the crate and the P3 surface. P2-LINT: gc-lint and the debug check. P3: accessors in `define_own_property`. H22: realms for R5. Freeze at M0 exit.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R07] | Brimstone did a whole-runtime refactor to GC-safe handles at month 6; Nova's code is "filled to the brim" with bind/unbind calls, and its author regrets it | [07-rust-engine-landscape.md](../research/07-rust-engine-landscape.md) |
| [R05] | A direct value in a native frame is a root only under CSS: V8 says "do not use direct handles without conservative stack scanning" | [05-gc-design.md](../research/05-gc-design.md) |
| [M] | Sizes, the throw protocol, LIFO panics and GC-stress survival of builder values: `tests::abi` (15 tests); test262: 0 new failures, language/ 8,683 | this ADR, 2026-10-21 |

## Validation

- `cargo test -p ep9_vm` (`tests::abi`), test262 exec and its GC-stress run on 4 targets; from P2, `cargo xtask gc-lint` in CI.
- The freeze review at M0 exit checks every row above against the built builtins (P3 templates).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H20-Q1 | Where does `new.target` live? (= H14-Q1.) v0: the feedback slot of a `[[Construct]]` frame; `Args::new_target` reads it only when the construct bit is set | M1 | A register filled at entry (H14-Q1) |
| H20-Q2 | The realm of a builtin with N realms (with H22-Q3, Q7) | M1 (`$262.createRealm`) | The callee's realm, read through `Args::callee` |
| H20-Q3 | `call`/`apply` without the copy | M1 | One copy now (`call_args`, `ArgsBuilder` into the callee frame); a tail-call host path in M1, as V8's Function.prototype.call shifts the arguments and tail-calls |
| H20-Q4 | The embedder API (`Isolate`) still hands out raw `Value`s | M0 exit | A `Cx` scope on `Isolate`, so embedders use `Local` too |

## Reopens if

- A GC-stress failure is traced to a `Local` in Rust heap memory: add a type-aware dylint lint (ABI-5 e).
- The JIT/AOT host-call stub needs more than 4 integer arguments on Win64.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft: signature, `Args`, `Local`, R1-R6, ObjectOps v0, lint plan, host-call stub, Q1-Q4 (M0 wk4 plan §4 P1-ABI; decisions W4-ABI-*) | P1-ABI agent |
