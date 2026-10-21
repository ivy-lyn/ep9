# H17: Object layout

| Field | Value |
|---|---|
| Status | Draft (2026-10-21; v0 as built in M0 wk4). Not frozen: see "Decide before M1a" |
| Fix by | M1 |
| Freeze point | M1a |
| Origin | v1#13; plan status U |
| Crates | `ep9_vm` (`object.rs`, `props.rs`, `shape.rs`, `exotic.rs`, `property.rs`, `accessor.rs`); `ep9_heap` (shape space, word0); later `ep9_baseline`, `ep9_opt` (shape guards, field loads) |
| Owner (plan role) | VM lead |
| Validated by | `ep9_vm` tests `tests/objects.rs`, `tests/shapes.rs`, `tests/accessors.rs` (GC stress + heap verifier); test262 with and without `--gc-stress 1` |
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
| ITERATOR | +24 iterated (array iterator: an object, null while its `next` runs user code, undefined when done; string iterator: a string, "" when done), +32 next | 0 |
| PRIMITIVE_WRAPPER | +24 value | 0 |
| BOUND_FUNCTION | +24 target, +32 this, +40 args | 2 |

| Item | v0 rule | Where |
|---|---|---|
| Shape of an object | `(o & !(CAGE_ALIGN - 1)) \| word0.shape_id`: no heap pointer needed. `ep9_vm` asks for a 64 GiB aligned cage of at most 64 GiB (`shape::heap_config`) | `shape::shape_of`, decision W4-OBJ-CAGE |
| Field i | `i < capacity`: in-object slot i; else `SLOTS[i - capacity]` (grows to max(need, 2 x old, 4)) | `props::field_ref` |
| Fast shapes | Transition tree; one descriptor array shared along a chain (prefix rule); hash index above 8 keys; weak transitions | `shape.rs` (P1-SHAPE) |
| Dictionary mode (OM-10) | Delete that is not the last; attribute change (data <-> accessor too); array-index key on a non-array; SetPrototypeOf with own properties; PreventExtensions; 1020 properties; 64 live transitions. One shared dictionary shape per (prototype, kind, extensible); a prototype object gets a unique one | `props::to_dictionary` |
| Prototype mode | When an object becomes a `[[Prototype]]` on any path (a new object, SetPrototypeOf of any object, a prototype included): is-prototype flag bit, a unique shape (no parent, no transitions) whose +40 holds the object's PROTO_INFO (its root and dictionary shapes). So every shape's proto is null or a prototype-mode object | `shape::proto_info_for`, `shape::with_proto` |
| Epoch (OM-8) | Any structural change of a prototype bumps `ShapeState::proto_epoch`: a named property or an array element that appears or goes away (a `length` cut that removes an element too), an attribute change, SetPrototypeOf, PreventExtensions, a dictionary move. A value store does not (a named or element value, `length` growth, a `length` cut over holes only). Only a prototype pays more than a flag test. The epoch saturates at `u32::MAX` and never wraps (W4-P3FIX-EPOCH-SAT; H16 K18) | `shape::note_change`, `shape::EPOCH_SATURATED`; elements: `object::array_set`, `object::array_set_length` (the only element writers) |
| Index keys on prototypes | A sticky per-runtime flag (`shape::proto_index_keys`) is set when a prototype-mode object gets an array-index own property (any attributes), or when an object that may have one becomes a prototype. While it is clear, `[[Set]]` and `Array.prototype.push` add a missing array element with no prototype walk; once set, they walk the chain (§10.1.9.2). It is never cleared, like V8's protector (W4-P3FIX-ELEM-FLAG, -ANY-INDEX, -PROTO-SCAN, -PUSH) | `shape::note_proto_index_key`, `shape::proto_info_for`, `property::array_store` |
| Extensible, `[[Prototype]]` | In the shape (flag EXTENSIBLE, +8 proto). A fast shape is always extensible | `property::prevent_extensions` |
| Keys | Stored keys are atoms or symbols (`key::intern_key` on every define path); a lookup classifies its probe once | W4-KEY-EQ |
| Accessors | Attribute bit 8 (`attr::ACCESSOR`) in descriptors and dictionary entries; the value slot holds an immutable ACCESSOR_PAIR (see "Accessors" below) | `accessor.rs`, `object::attr` |
| Functions | `length`, `name` are own data properties {w: false, e: false, c: true} in the 2 slots. `prototype` is made on first use, and before any later key is added, so the key order is right | `object::new_function`, `exotic.rs` |

### Accessors (M0 wk4, P3-ACC)

JSC style (plan §2 row 6): an accessor is an attribute bit plus an immutable pair cell in the
ordinary value slot, so a shape describes an accessor exactly as it describes a data field.

```text
 descriptor details: attrs = ACCESSOR | e | c (never WRITABLE), field i
 field i (in-object, SLOTS or PROPS entry) ──> ACCESSOR_PAIR (kind 67, 24 B): +8 getter, +16 setter
                                               (a callable, or undefined when absent)
```

| Item | v0 rule | Where |
|---|---|---|
| Pair | Immutable. `[[DefineOwnProperty]]` builds a new pair from the descriptor and the old halves (6.c keeps a half the descriptor does not name) and stores it in place of the old one | `accessor::new_pair`, `property::define_own_property` |
| `[[Get]]` | The first property on the chain decides; an accessor calls its getter with the receiver as `this` (a primitive base passes itself), undefined without a getter | `property::get_with_receiver`, `accessor::call_getter` |
| `[[Set]]` | The first property on the chain decides; an accessor calls its setter with the receiver, or fails without one (sloppy: ignored; strict: TypeError "Cannot set property x of #<Object> which has only a getter"). A primitive base walks its prototype's chain (§6.2.5.6 step 5.a) | `property::set_with_receiver`, `property::set` |
| `[[DefineOwnProperty]]` | ValidateAndApplyPropertyDescriptor for both kinds: steps 5.a-e, then 6.a (data to accessor), 6.b (accessor to data, `writable` false unless given), 6.c (field by field). Data <-> accessor is an attribute change, so a fast object goes to dictionary mode (OM-10) | `property::validate`, `property::define_own_property` |
| Epoch | A new pair in place of an old one bumps the epoch of a prototype, even with the same attributes (an IC may cache a getter found on a prototype while the epoch holds) | `props::own_define` |
| Raw reads | `props::own_get`, `own_entry`, `own_value_or_proto` return the pair of an accessor; a caller tests `attr::ACCESSOR` before it uses the value as a JS value. A pair never reaches JS | `props.rs` |
| Engine text | An error's "Name: message" header (made inside `throw_error`, also on stack overflow) and the reported constructor name read data properties only: no getter runs there | `property::get_data_from` |
| Exotic | An accessor on an array element or array `length` is not built: an element throws "not implemented" (H18), `length` rejects it (ArraySetLength step 1). String wrapper properties are not configurable, so they reject it | `property::define_own_property` |
| Runtime ids | 12 `DefineAccessorInLiteral` (enumerable) and 16 `DefineClassAccessor` (not enumerable, sets `[[HomeObject]]`): a descriptor with the one present half, DefinePropertyOrThrow; an anonymous function (computed key) is named `"get k"`, `"set k"` or `"get [desc]"` | `interp/slow/runtime.rs` |

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
| An ACCESSOR_PAIR never changes; a define makes a new one, and a new pair on a prototype bumps the epoch | `validate_and_apply_accessor_descriptors`, `replacing_an_accessor_of_a_prototype_bumps_the_epoch` | M0 wk4 (P3-ACC) |
| Getters and setters get the receiver as `this` (inherited, `super`, primitive bases) | `literal_getters_and_setters_take_the_receiver_as_this`, `keyed_super_getters_and_setters_see_this`, `getters_and_setters_receive_the_receiver_also_a_primitive` | M0 wk4 (P3-ACC) |
| Engine-made error text runs no getter | `engine_error_text_runs_no_getters` | M0 wk4 (P3-ACC) |

## Consequences

- **Gains.** A shape guard is one u32 compare, and the field offset comes from the shape. `shape_of` needs no runtime pointer, so host paths (`proto_of`, `is_extensible`) stay free functions.
- **Costs.** The 64 GiB cage alignment leaves 2,048 cage slots below 2^47. Prototype objects never share shapes. Class constructors with `extends` go to dictionary mode (SetPrototypeOf with own properties).
- **Follow-ups.** P3-IC built the (shape, field) caches (H16 K9-K18). Exotic own properties (array `length` and elements, a constructor's lazy `prototype`, String wrapper properties) are not in shapes: the epoch covers an element that appears or goes away, but a holder's exotic property is never a field load. P3-ACC built accessors (section above); the super paths pass the receiver (`LoadKeyedFromSuper`, `StoreToSuper`; opcode 0x22 with P3-IC). An IC must not make a data handler for an `ACCESSOR` descriptor: the slot holds the pair. Slack tracking replaces the fixed `new F` capacity of 8.

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
| Accessor ICs | Getter/setter call handlers (V8 LoadHandler for accessors, JSC GetterSetter cached by identity): cache the pair or the getter, under the epoch or a per-holder validity cell? Data <-> accessor goes to dictionary mode today; does a fast object need an accessor transition instead? | Accessors take the IC slow path; any data <-> accessor change goes to dictionary mode |
| Element accessors | Accessor elements (and non-default element attributes) need a sparse or dictionary elements kind (H18) | "not implemented" |

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
| 2026-10-21 | fill | Accessors as built: ACCESSOR_PAIR (kind 67, immutable), `[[Get]]`/`[[Set]]` with the receiver, ValidateAndApplyPropertyDescriptor for accessors, the pair epoch rule, data-only engine text, runtime ids 12 and 16; two decide-before-M1a rows (accessor ICs, element accessors) | P3-ACC agent |
| 2026-10-21 | fill | As built after the P3 review: the "Index keys on prototypes" row (sticky `proto_index_keys` flag, W4-P3FIX-ELEM-*), and the ITERATOR states (null = executing, W4-P3FIX-ITER-EXEC-NULL) | P4-DOCS agent |
| 2026-10-21 | clarify | Epoch row names the saturation (W4-P3FIX-EPOCH-SAT, already H16 K18); Follow-ups say the IC caches are built | P4-DOCS agent |
| 2026-10-21 | rewrite (version) | OBJECT_LAYOUT 1 -> 2 (`versions.rs`), as built in M2-P0: kind 23 GENERATOR, fixed fields +24 state, +32 mode, +40 function, +48 receiver, +56 context, +64 registers (an ELEMENTS cell), +72 resume mode, +80 async slot, capacity 0 (`object::genf`); kind 24 SLOT_OBJECT, fixed field +24 ObjectClass id, its in-object slots are the internal slots (`object::slot_obj`); ITERATOR gets +40 kind (values 0, keys 1, entries 2; `iter::FIXED` 2 -> 3, the cell grows 40 -> 48 B); the CONTEXT +16 word is a native word that points at the unit's `ContextScope { unit, scope }` record (C5; `object::context_scope`), not an int32 scope id. The Decision kind table still shows layout 1 for ITERATOR; object.rs has the v2 table | P0-VM agent |
