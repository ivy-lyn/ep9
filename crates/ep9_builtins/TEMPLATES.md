# Writing a builtin

How to add an ECMA-262 builtin to `ep9_builtins`. The ABI is H20
([docs/adr/H20-builtin-abi.md](../../docs/adr/H20-builtin-abi.md)). Every `rust` block below is a
doctest (`cargo test -p ep9_builtins --doc`), so it compiles against today's ABI.

## 1. Where things go

```text
ep9_builtins::TABLE = MODULES in install order
  object(1) function(2) array(3) number(4) boolean(5) string(6) symbol(7) math(8) global(9)
     |
     src/<module>.rs
       BLOCK       the module's block of builtin ids (fixed, never changes)
       DEFS        the functions; DEFS[i] has the id BLOCK.i (append-only)
       INTRINSICS  the intrinsics the module makes (%Number%, %Math%, ...)
       PROPS       the properties it adds, per object (target = an IntrinsicId)
       MODULE      the four above
     tests/<module>.rs                JS checks (helpers: tests/common/mod.rs)
     tests/golden/ids-<module>.txt    the id golden (tests/table.rs)

Isolate::new
  check the tables (once per table)
  realm init: %Object.prototype%, %Function.prototype%, the other prototypes, the global
  install the VM table (block 0: Object, Function, Array, Error, ...), then TABLE:
    pass 1  one function object per DEFS entry
    pass 2  the INTRINSICS
    pass 3  the PROPS, module by module, in table order (= property order)
```

The VM table (`crates/ep9_vm/src/natives.rs`, block 0) already makes `%Object%`, `%Function%`,
`%Array%`, the error constructors, `Object.prototype.toString`/`valueOf`, `Array.prototype`
`join`/`toString`/`push` and the globals `globalThis`, `undefined`, `NaN`, `Infinity`.
`check_tables` rejects a second maker of an intrinsic and a key defined twice on one object.

## 2. The function

```text
fn(cx: &mut Cx<'s>, this: Local<'s>, args: Args<'s>, new_target: Local<'s>) -> JsResult<Local<'s>>
```

| Part | What it is |
|---|---|
| `cx` | everything the builtin may do: ObjectOps (`get`, `set`, `define_property_or_throw`, ...), conversions (`to_number`, `to_string`, `to_object`, ...), calls, `intrinsic(id)`, `throw_*` |
| `this` | the this value, as passed (not coerced) |
| `args` | the arguments: `args.get(i)` (undefined past the end), `args.len()`, `args.iter()`, `args.rest(i)`, `args.callee()` |
| `new_target` | undefined for a call; the new.target of a `new` |
| result | `Ok(value)`, or `Err(throw)` from `cx.throw_*`, `cx.not_implemented` or a `?` |

Write the spec steps as comments, one per step, and keep their numbers. A `?` in the spec is a `?`
in Rust; a `!` is a call whose `Err` cannot happen (pass it on with `?` anyway).

### Template 1: a method

```rust
use ep9_vm::{Args, Cx, JsResult, Local};

/// Object.prototype.hasOwnProperty ( V ) (§20.1.3.2).
fn has_own_property<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let P be ? ToPropertyKey(V).
    let p = cx.to_property_key(args.get(0))?;
    // 2. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?;
    // 3. Return ? HasOwnProperty(O, P).
    Ok(Local::boolean(cx.has_own_property(o, p)?))
}
```

### Template 2: a method of a primitive prototype

`this_primitive_value` is thisBooleanValue, thisNumberValue, thisStringValue and
thisSymbolValue in one: the primitive, or the one inside a wrapper of that type, else a
TypeError.

```rust
use ep9_vm::{Args, Cx, JsResult, Local, PrimitiveType};

/// Boolean.prototype.valueOf ( ) (§20.3.3.3).
fn boolean_value_of<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return ? thisBooleanValue(this value).
    cx.this_primitive_value(this, PrimitiveType::Boolean)
}
```

### Template 3: a constructor

A call has `new_target` undefined. For `new`, the object's prototype comes from new.target
(a subclass passes its own), with this module's prototype as the default.

```rust
use ep9_vm::{Args, Cx, IntrinsicId, JsResult, Local};

/// Boolean ( value ) (§20.3.1.1).
fn boolean<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let b be ToBoolean(value).
    let b = Local::boolean(cx.to_boolean(args.get(0)));
    // 2. If NewTarget is undefined, return b.
    let Some(nt) = new_target.as_object() else {
        return Ok(b);
    };
    // 3. Let O be ? OrdinaryCreateFromConstructor(NewTarget, "%Boolean.prototype%",
    //    « [[BooleanData]] »).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::BooleanPrototype)?;
    // 4. Set O.[[BooleanData]] to b.
    let o = cx.new_wrapper(proto, b)?;
    // 5. Return O.
    Ok(o.value())
}
```

A plain object (no internal slot) is `cx.ordinary_create_from_constructor(nt, default)`.

### Template 4: all the arguments

Read the arguments through `args` (they stay rooted in the frame), convert them all first, then
compute: a conversion can run JS (`valueOf`), and the spec orders those calls.

```rust
use ep9_vm::{Args, Cx, JsResult, Local};

/// Math.max ( ...args ) (§21.3.2.24).
fn math_max<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let coerced be a new empty List.
    // 2. For each element arg of args, do
    //    a. Let n be ? ToNumber(arg).
    //    b. Append n to coerced.
    // (numbers are plain f64: no list of Locals is needed)
    let mut coerced = Vec::with_capacity(args.len());
    for arg in args.iter() {
        coerced.push(cx.to_number(arg)?);
    }
    // 3. Let highest be -∞𝔽.
    let mut highest = f64::NEG_INFINITY;
    // 4. For each element number of coerced, do
    for number in coerced {
        // a. If number is NaN, return NaN.
        if number.is_nan() {
            return Ok(Local::number(f64::NAN));
        }
        // b. If number is +0𝔽 and highest is -0𝔽, set highest to +0𝔽.
        if number == 0.0 && highest == 0.0 && highest.is_sign_negative() {
            highest = 0.0;
        }
        // c. If number > highest, set highest to number.
        if number > highest {
            highest = number;
        }
    }
    // 5. Return highest.
    Ok(Local::number(highest))
}
```

A list of JS values is a `ScratchList` (`cx.scratch_list()`), a JS array, or an `ArgsBuilder`
(`cx.args_builder()`, for call arguments): never a `Vec<Local>` (rule R2 below).

### Template 5: a getter, a symbol key

```rust
use ep9_vm::{Args, Cx, JsResult, Local, PrimitiveType};

/// get Symbol.prototype.description (§20.4.3.2).
fn symbol_description<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let s be the this value.
    // 2. Let sym be ? thisSymbolValue(s).
    let sym = cx.this_primitive_value(this, PrimitiveType::Symbol)?;
    let sym = sym.as_symbol().expect("thisSymbolValue gives a symbol");
    // 3. Return sym.[[Description]].
    Ok(cx.symbol_description(sym))
}
```

Other ABI helpers for the P3 builtins: `symbol_for` and `key_for` (`Symbol.for`/`keyFor`),
`descriptive_string` (`String(sym)`), `set_function_name` and `set_function_length` (`bind`),
`new_bound_function`, `create_list_from_array_like` (`apply`), `to_property_descriptor` and
`from_property_descriptor` (`defineProperty`, `getOwnPropertyDescriptor`).

## 3. Add it to the table

```rust
use ep9_vm::{
    Atom, BuiltinDef, BuiltinModule, BuiltinTable, ConstValue, IntrinsicDef, IntrinsicId,
    IntrinsicValue, ObjectProps, PropAttrs, PropDef, PropKey, WellKnownSymbol,
};
# use ep9_vm::{Args, Cx, JsResult, Local};
# // The functions of section 2 (bodies left out here).
# fn boolean<'s>(_: &mut Cx<'s>, _: Local<'s>, _: Args<'s>, _: Local<'s>) -> JsResult<Local<'s>> { todo!() }
# fn boolean_value_of<'s>(_: &mut Cx<'s>, _: Local<'s>, _: Args<'s>, _: Local<'s>) -> JsResult<Local<'s>> { todo!() }
# fn math_max<'s>(_: &mut Cx<'s>, _: Local<'s>, _: Args<'s>, _: Local<'s>) -> JsResult<Local<'s>> { todo!() }
# fn symbol_description<'s>(_: &mut Cx<'s>, _: Local<'s>, _: Args<'s>, _: Local<'s>) -> JsResult<Local<'s>> { todo!() }

pub const BLOCK: u8 = 120; // a real module keeps its block (see src/lib.rs)

// DEFS[i] is the builtin BLOCK.i: append only, never reorder, remove or reuse a line.
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Boolean", 1, boolean),        // 0
    BuiltinDef::method("valueOf", 0, boolean_value_of),    // 1
    BuiltinDef::method("max", 2, math_max),                // 2
    BuiltinDef::getter("description", symbol_description), // 3: named "get description"
];

pub const INTRINSICS: &[IntrinsicDef] = &[
    // %Boolean% is the function object of DEFS[0].
    IntrinsicDef { id: IntrinsicId::Boolean, value: IntrinsicValue::Builtin(0) },
    // %Math% is a new ordinary object.
    IntrinsicDef {
        id: IntrinsicId::Math,
        value: IntrinsicValue::Object { proto: Some(IntrinsicId::ObjectPrototype) },
    },
];

pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Boolean,
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::BooleanPrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::BooleanPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::Boolean,
                PropAttrs::METHOD,
            ),
            PropDef::method(PropKey::Atom(Atom::ValueOf), 1),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Math,
        props: &[
            PropDef::method(PropKey::Str("max"), 2),
            PropDef::constant(
                PropKey::Str("E"),
                ConstValue::Number(core::f64::consts::E),
                PropAttrs::NONE,
            ),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Math"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::SymbolPrototype,
        props: &[PropDef::accessor(PropKey::Str("description"), Some(3), None)],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        props: &[
            PropDef::intrinsic(PropKey::Str("Boolean"), IntrinsicId::Boolean, PropAttrs::METHOD),
            PropDef::intrinsic(PropKey::Str("Math"), IntrinsicId::Math, PropAttrs::METHOD),
        ],
    },
];

pub const MODULE: BuiltinModule = BuiltinModule {
    name: "example",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// The checks that `Isolate::new` runs (blocks, ids, kinds, keys, intrinsics):
static TABLE: BuiltinTable = BuiltinTable::new("example", &[MODULE]);
assert_eq!(ep9_vm::check_tables(&[ep9_vm::internals::VM_BUILTINS, &TABLE]), Ok(()));
```

| Entry | Use it for |
|---|---|
| `BuiltinDef::method(name, length, f)` / `constructor` / `getter` / `setter` | a function; `length` and `name` are its own properties; a getter is named "get x" |
| `PropDef::method(key, i)` | `key` = the function `DEFS[i]`, {w, c} |
| `PropDef::intrinsic(key, id, attrs)` | `constructor`, `prototype`, a global constructor or namespace |
| `PropDef::constant(key, value, attrs)` | `Math.PI` ({}), `NaN` ({}), `@@toStringTag` ({c}) |
| `PropDef::alias(key, of, attrs)` | one function under two names: `[Symbol.iterator]` = `values`; `of` must come earlier on the same object |
| `PropDef::accessor(key, get, set)` | an accessor ({c}); needs accessor properties (P3-ACC) |
| `IntrinsicValue::Builtin(i)` / `Object { proto }` | an intrinsic that is `DEFS[i]` / a new ordinary object |

Keys: `PropKey::Atom` for a name of the `atoms!` list (`crates/ep9_vm/src/atoms.rs`, no lookup at
init), `PropKey::Str` for any other name, `PropKey::Symbol` for a well-known symbol. Attributes
follow §18: functions and data {w, c} (`METHOD`), value properties and `prototype` {} (`NONE`),
`@@toStringTag` and accessors {c} (`CONFIGURABLE`).

Constructor rule (checked by `check_tables`): the first key a table gives a constructor that it
makes (`IntrinsicValue::Builtin` of a `constructor` def) must be `prototype`, a data property with
attributes {} (`NONE`), and every such constructor must have one. The keys then read `length`,
`name`, `prototype`, then the rest, as in V8. Do not give a constructor a `length` or `name` key:
the function object already has them, and install fails with `IsolateError::BuiltinTable`.

Then bless the id golden and read the diff: only appended lines.

```sh
EP9_BLESS=1 cargo test -p ep9_builtins --test table ids_match_the_goldens
git diff crates/ep9_builtins/tests/golden
```

## 4. GC and safety rules

| Rule | Do | Checked by |
|---|---|---|
| R1 | keep JS values in `Local`s in Rust locals or in `args`: the collector finds them on the stack | GC-stress tests (`run_both`), test262 `--gc-stress 1` |
| R2 | no `Local`, `Args`, `Rest`, `PropertyDescriptor`, `ArgsBuilder` or `ScratchList` in a `Vec`, `Box`, `HashMap`, struct field on the heap, `static` or `thread_local` | lifetimes and `!Send`; `cargo xtask gc-lint`; a debug check in `Local` |
| R3 | a string view (`FlatStr`, `code_unit_at`) borrows `&Cx`; finish with it before the next `&mut Cx` call | the borrow checker |
| R4 | return `Err` only from `cx.throw_*`, `cx.not_implemented` or a `?`; never `Ok` after a throw | debug asserts in the host call |
| R5 | make objects with `cx` (`new_object`, `intrinsic`, ...); never keep one in a static | one realm in v0; `args.callee()` for N realms (H20-Q2) |
| R6 | read `length`, indices, `name` and `prototype` through ObjectOps; they are virtual and get spec attributes | `get_own_property` |
| crate | no `unsafe` (`#![forbid(unsafe_code)]`); no `ep9_value::Value` or heap cell type | the compiler; `clippy.toml` `disallowed-types` |
| crate | not done yet: `Err(cx.not_implemented("what"))` gives "not implemented in ep9 v0: what" | test262 failure causes |

## 5. Test it

- **Unit tests** in `tests/<module>.rs` (the P3 owner adds the file):

  ```rust,ignore
  mod common;

  #[test]
  fn boolean_value_of() {
      // run_both: with and without GC stress; both must print the same lines.
      assert_eq!(common::run_both("print(true.valueOf())"), Ok(vec!["true".to_string()]));
  }
  ```

  `common::run(src, stress)` runs once; `common::isolate(stress)` gives an isolate with `TABLE` and
  `print`, and `Isolate::with_cx` reads results as `Local`s.
- **test262 subset** (exit 1 = differs from the expectation file; read the NOW PASSING and NEW
  FAILURE lines):

  ```sh
  cargo run --release --locked -p ep9_test262 -- exec test/built-ins/Boolean
  cargo run --release --locked -p ep9_test262 -- exec --gc-stress 1 --timeout 60 test/built-ins/Boolean
  ```

- **Before a commit:** `cargo clippy -p ep9_builtins --all-targets --locked -- -D warnings`,
  `cargo test -p ep9_builtins`, and the full test262 run with and without GC stress
  (`--skip-list expectations/test262-gc-skip.txt` with `--gc-stress 1`).
