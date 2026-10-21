# H17: Object layout

| Field | Value |
|---|---|
| Status | Draft (2026-10-21; v0 as built in M0 wk4). Not frozen: see "Decide before M1a" |
| Fix by | M1 |
| Freeze point | M1a |
| Origin | v1#13; plan status U |
| Crates | `ep9_vm` (`object.rs`, `props.rs`, `shape.rs`, `exotic.rs`, `property.rs`); `ep9_heap` (shape space, word0); later `ep9_baseline`, `ep9_opt` (shape guards, field loads) |
| Owner (plan role) | VM lead |
| Validated by | `ep9_vm` tests `tests/objects.rs`, `tests/shapes.rs` (GC stress + heap verifier); test262 with and without `--gc-stress 1` |
| Plan sources | [02 §6 H17](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [06 §2](../plan/06-object-model-and-spec.md#2-header-and-objects-h8-extended), [06 §2.2](../plan/06-object-model-and-spec.md#22-shapes-h17) |

## Context

- A monomorphic property load must be one shape guard and one load (02 §6 H17). So the shape must say where each field is, and the object must not need a second lookup.
- Depends on H8 (word0, ShapeId = offset in the shape space), H9/H11 (barrier v2 for word0), H18 (elements), H19 (atom keys). H16 (ICs) and the JIT/AOT tiers depend on it.

## Decision

v0 (M0 wk4) builds the plan's JSObject header with shapes in the shape space. The plan items marked "decide before M1a" below are not built.

```text
 +0  word0   shape_id u32 | gc_state | kind | flags u16 (bits 0-7 in-object slot count, bit 8 is-prototype)
 +8  SLOTS (fast, out-of-object fields) | PROPS (dictionary table) | undefined
 +16 ELEMENTS | undefined
 +24 fixed fields of the kind, then in-object slots [capacity]

 shape (64 B, Space::Shape):  +8 proto  +16 parent  +24 descriptors  +32 transitions
                              +40 transition table | PROTO_INFO (unique)  +48 counts  +56 flags
```

| Kind | Fixed fields | Default capacity |
|---|---|---|
| OBJECT | none | 4 for `{}`; n for a literal with n named keys; 8 for `new F` and other prototypes |
| FUNCTION | +24 info, +32 context, +40 home | 2 (`length`, `name`) |
| ARRAY | +24 length | 0 |
| ERROR | +24 unit, +32 offset | 2 |
| ITERATOR | +24 iterated, +32 next | 0 |
| PRIMITIVE_WRAPPER | +24 value | 0 |
| BOUND_FUNCTION | +24 target, +32 this, +40 args | 2 |

| Item | v0 rule | Where |
|---|---|---|
| Shape of an object | `(o & !(CAGE_ALIGN - 1)) \| word0.shape_id`: no heap pointer needed. `ep9_vm` asks for a 64 GiB aligned cage of at most 64 GiB (`shape::heap_config`) | `shape::shape_of`, decision W4-OBJ-CAGE |
| Field i | `i < capacity`: in-object slot i; else `SLOTS[i - capacity]` (grows to max(need, 2 x old, 4)) | `props::field_ref` |
| Fast shapes | Transition tree; one descriptor array shared along a chain (prefix rule); hash index above 8 keys; weak transitions | `shape.rs` (P1-SHAPE) |
| Dictionary mode (OM-10) | Delete that is not the last; attribute change (data <-> accessor too); array-index key on a non-array; SetPrototypeOf with own properties; PreventExtensions; 1020 properties; 64 live transitions. One shared dictionary shape per (prototype, kind, extensible); a prototype object gets a unique one | `props::to_dictionary` |
| Prototype mode | When an object becomes a `[[Prototype]]` on any path (a new object, SetPrototypeOf of any object, a prototype included): is-prototype flag bit, a unique shape (no parent, no transitions) whose +40 holds the object's PROTO_INFO (its root and dictionary shapes). So every shape's proto is null or a prototype-mode object | `shape::proto_info_for`, `shape::with_proto` |
| Epoch (OM-8) | Any structural change of a prototype bumps `ShapeState::proto_epoch`: a named property or an array element that appears or goes away (a `length` cut that removes an element too), an attribute change, SetPrototypeOf, PreventExtensions, a dictionary move. A value store does not (a named or element value, `length` growth, a `length` cut over holes only). Only a prototype pays more than a flag test | `shape::note_change`; elements: `object::array_set`, `object::array_set_length` (the only element writers) |
| Extensible, `[[Prototype]]` | In the shape (flag EXTENSIBLE, +8 proto). A fast shape is always extensible | `property::prevent_extensions` |
| Keys | Stored keys are atoms or symbols (`key::intern_key` on every define path); a lookup classifies its probe once | W4-KEY-EQ |
| Accessors | Attribute bit 8 (`attr::ACCESSOR`) in descriptors and dictionary entries. Getters and setters are called from P3-ACC on | `object::attr` |
| Functions | `length`, `name` are own data properties {w: false, e: false, c: true} in the 2 slots. `prototype` is made on first use, and before any later key is added, so the key order is right | `object::new_function`, `exotic.rs` |

## Rules

| Rule | Check | From |
|---|---|---|
| Every JS object has a non-zero word0 ShapeId that names a live shape | Heap verifier after every GC (GC stress, test262 `--gc-stress 1`) | M0 wk4 |
| word0 changes only through `shape::install` (`ep9_heap::set_shape`, barrier v2) | Code review; `grep set_shape crates/ep9_vm` | M0 wk4 |
| The same add order from the same root gives the same shape | `two_literals_with_the_same_keys_share_a_shape_id` | M0 wk4 |
| Deleting the last property goes back to the parent shape | `deleting_the_last_property_goes_back_to_the_parent_shape` | M0 wk4 |
| Each OM-10 trigger moves to dictionary mode | `om10_*` tests in `tests/objects.rs` | M0 wk4 |
| `[[OwnPropertyKeys]]` order: indices, strings in creation order, symbols | `object_keys_and_for_in_order_survive_layout_changes`, `a_lazy_prototype_keeps_its_place_in_the_key_order` | M0 wk4 |
| No content compare of keys in UNIT(ep9_vm) | `EP9_ASSERT_ATOM_KEYS=1 cargo test -p ep9_vm` | M0 wk4 |
| Every shape's proto is null or a prototype-mode object | `debug_assert` in `shape::new_shape`; `set_prototype_of_a_prototype_puts_the_new_prototype_in_prototype_mode` (fast and dictionary objects) | M0 wk4 (P2 fix) |
| A structural change of a prototype bumps the epoch; a value store does not (named and element paths, fast and dictionary prototypes) | `an_object_used_as_a_prototype_gets_a_unique_shape_and_bumps_the_epoch`, `dense_element_changes_of_an_array_prototype_bump_the_epoch` | M0 wk4 (P2 fix) |

## Consequences

- **Gains.** A shape guard is one u32 compare, and the field offset comes from the shape. `shape_of` needs no runtime pointer, so host paths (`proto_of`, `is_extensible`) stay free functions.
- **Costs.** The 64 GiB cage alignment leaves 2,048 cage slots below 2^47. Prototype objects never share shapes. Class constructors with `extends` go to dictionary mode (SetPrototypeOf with own properties).
- **Follow-ups.** P3-IC caches (shape, field). Exotic own properties (array `length` and elements, a constructor's lazy `prototype`, String wrapper properties) are not in shapes: the epoch covers an element that appears or goes away, but a holder's exotic property is never a field load. P3-ACC calls getters and setters and passes the receiver on the super paths. Slack tracking replaces the fixed `new F` capacity of 8.

## Decide before M1a

| Item | Question | v0 today |
|---|---|---|
| Slack tracking | Per-constructor in-object capacity from the first N instances (06 §2.2) | Fixed 4 / n / 8 / 2 |
| Representation and const bits | Field representation (Smi, double, heap) and the constness bit + replacement watchpoints as InvalidatingFuses (H31) | Only attribute bits |
| Elements for every kind | Which kinds have ELEMENTS; index keys on non-arrays in elements, not dictionary mode (H18) | Arrays only; index keys -> dictionary |
| Transition hashing | Transition table form past one child; the 64-child cap | Linear table, cap 64 |
| Validity cells | Prototype-chain validity cells per PROTO_INFO in place of one global epoch | One `proto_epoch` |
| S2 weak transition child | Reading a weak transition child is safe because marking is stop-the-world. With incremental marking, a weak child that is read and stored must be marked by the barrier or the final rescan | Stop-the-world marking |
| Dictionary mode | Shared dictionary shapes per (prototype, kind, extensible): is a per-object shape needed for IC misses? The PROPS table is scanned linearly: when does it get a hash index? | Shared shapes; linear scan |

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R06] | Transition trees, shared descriptor arrays, prototype mode and dictionary mode (V8 HTC5) | [06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R17] | Run-local shapes need no deterministic layout (RW-cell guards) | [17-aot-refutation.md](../research/17-aot-refutation.md) |

## Validation

- UNIT(ep9_vm) with GC stress 1 and the heap verifier: `tests/objects.rs`, `tests/shapes.rs`.
- test262 exec and exec `--gc-stress 1`: 0 NEW FAILURE.
- M1a: the SP-Facts and IC work check the guard + load path (H16).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H17-Q1 | Every row of "Decide before M1a" | M1a | v0 as built |
| H17-Q2 | Is 64 GiB cage alignment kept, or does `shape_of` read the cage base from the runtime? | M1a | Keep: it saves a load in every host path |

## Reopens if

- An IC or JIT measurement shows the shared dictionary shapes or prototype mode cost more than the plan's budget.
- The cage must be larger than 64 GiB.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft: v0 layout as built (word0 shapes, fixed fields, capacity, OM-10, prototype mode, epoch, function `length`/`name`), decide-before-M1a list incl. the S2 weak-child note | P2-OBJ agent |
| 2026-10-21 | fix | Epoch rule on every path: SetPrototypeOf of a prototype puts the new prototype in prototype mode; dense element adds, deletes and `length` cuts of a prototype bump the epoch; the proto-mode invariant is asserted in `new_shape` | P2 fix agent (review W4-P2FIX-EPOCH) |
