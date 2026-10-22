//! The host functions of the VM's own builtin table (`VM_TABLE`, block 0), on the builtin ABI
//! v1 (H20). They live in ep9_vm because they read VM internals (iterator cells, error sites)
//! or are needed before any table: `%Function.prototype%` is the prototype of every function.
//! The other builtins are in `ep9_builtins`.
//!
//! | Function | Spec |
//! |---|---|
//! | `%Function.prototype%` (callable, returns undefined), `Function` (CreateDynamicFunction through the compiler hook, `abi/script.rs`) | §20.2.3, §20.2.1.1 |
//! | `Object`, `Array` (with NewTarget: GetPrototypeFromConstructor) | §20.1.1.1, §23.1.1.1 |
//! | `Error` and the native errors | §20.5.1.1, §20.5.6.1.1 |
//! | `Object.prototype.toString`, `valueOf` (ToObject) | §20.1.3.6, §20.1.3.7 |
//! | `Error.prototype.toString` | §20.5.3.4 |
//! | `Array.prototype.join`, `toString` (ToObject, then `join` or %Object.prototype.toString%), `push` (generic: any object, through `[[Set]]`) | §23.1.3.18, §23.1.3.36, §23.1.3.23 |
//! | `Array.prototype.values` = `Array.prototype[@@iterator]`, `keys`, `entries`, %ArrayIteratorPrototype%.next | §23.1.3.38, §23.1.3.40, §23.1.3.19, §23.1.3.5, §23.1.5.2.1 |
//! | `Array.prototype[@@unscopables]` (a null-prototype object of 16 `true` keys; no function) | §23.1.3.41 |
//! | `String.prototype[@@iterator]`, %StringIteratorPrototype%.next | §22.1.3.36, §22.1.5.2.1 |
//! | `%Iterator.prototype%[@@iterator]`; `%Iterator.prototype%[@@toStringTag]` (an accessor: get "Iterator", set SetterThatIgnoresPrototypeProperties, ES2025) | §27.1.2.1, §27.1.4.14 |
//! | `Function.prototype[@@hasInstance]` (OrdinaryHasInstance; `instanceof` runs it without a call, `property::instance_of`) | §20.2.3.6 |
//! | `%GeneratorFunction%`, `%GeneratorPrototype%` `next`/`return`/`throw`, `%AsyncFunction%` (`generator.rs`; `%GeneratorFunction%` compiles through the compiler hook like `Function`, `abi/script.rs`; `%AsyncFunction%` is CreateDynamicFunction of kind async through the same hook, M3-P1 ASYNC) | §27.3.1.1, §27.5.1.2-4, §27.7.1.1 |
//! | `%eval%`, the global `eval` (indirect eval; direct eval is `interp/slow/eval.rs`), through the compiler hook (`abi/script.rs`) | §19.2.1 |
//!
//! The built-in iterators are ITERATOR cells (object.rs): the iterated value, the next index
//! and the iteration kind (`object::iter_kind`). The iterated value tells an array iterator
//! from a string iterator, so each `next` checks its receiver (GeneratorValidate's brand)
//! from it; the kind field says what an array iterator yields:
//!
//! | Iterator | Iterated value | Kind | Yields | Done | Executing |
//! |---|---|---|---|---|---|
//! | array (`CreateArrayIterator`) | an object: an Array, or any array-like (`LengthOfArrayLike`) | values (`values`, `[@@iterator]`), keys (`keys`), entries (`entries`) | the element; the index; `[index, element]` | undefined | null, while `next` runs user code (getters, `valueOf`) |
//! | string (`CreateStringIterator`) | a string primitive | values | the code point | the empty string | never: its `next` runs no user code |
//!
//! A `next` on an executing iterator throws a TypeError (GeneratorValidate, §27.5.3.2).
//!
//! `interp/slow/iter.rs` (GetIterator, spread) makes the same cells (kind values) without
//! calling `values` or `[@@iterator]` while the method it finds is the intrinsic; the
//! iterators of `keys` and `entries` take the generic protocol there (their `[@@iterator]` is
//! `%Iterator.prototype%[@@iterator]`).

use ep9_value::JsResult;

use crate::abi::{Args, Cx as HostCx, Local};
use crate::atoms::Atom;
use crate::builtins::{
    self, BuiltinDef, BuiltinModule, BuiltinTable, ConstValue, IntrinsicDef, IntrinsicId,
    IntrinsicValue, ObjectProps, PropAttrs, PropDef, PropKey, PropValue, WellKnownSymbol,
};
use crate::conv::{self, num};
use crate::error;
use crate::frame::{FALSE, NULL, TRUE, UNDEFINED};
use crate::interp::Cx;
use crate::kinds::{is_callable, is_string};
use crate::object::{self, attr, is_array, is_object, own_define};
use crate::property;
use crate::runtime::{ErrorKind, Root, Runtime};
use crate::string::{self, MAX_STRING_LENGTH};
use crate::{generator, symbol, wrapper};

// ---- the VM table ------------------------------------------------------------------------------

/// The definition indices of the VM module (block 0). Append-only: the golden
/// `crates/ep9_vm/tests/golden/vm-builtin-ids.txt` pins them (H20, W4-BUILTIN-IDS).
pub(crate) mod vm {
    /// `%Function.prototype%` itself (made by `realm::init`, before every other function).
    pub const FUNCTION_PROTOTYPE: u16 = 0;
    pub const FUNCTION: u16 = 1;
    pub const OBJECT: u16 = 2;
    pub const OBJECT_TO_STRING: u16 = 3;
    pub const OBJECT_VALUE_OF: u16 = 4;
    pub const ARRAY: u16 = 5;
    pub const ARRAY_JOIN: u16 = 6;
    pub const ARRAY_TO_STRING: u16 = 7;
    pub const ARRAY_PUSH: u16 = 8;
    pub const ITERATOR_NEXT: u16 = 9;
    pub const ERROR: u16 = 10;
    pub const TYPE_ERROR: u16 = 11;
    pub const RANGE_ERROR: u16 = 12;
    pub const REFERENCE_ERROR: u16 = 13;
    pub const SYNTAX_ERROR: u16 = 14;
    pub const EVAL_ERROR: u16 = 15;
    pub const URI_ERROR: u16 = 16;
    pub const ERROR_TO_STRING: u16 = 17;
    /// `Array.prototype.values`, also `Array.prototype[@@iterator]` (one function object).
    pub const ARRAY_VALUES: u16 = 18;
    /// `String.prototype[@@iterator]`.
    pub const STRING_ITERATOR: u16 = 19;
    /// `%Iterator.prototype%[@@iterator]`.
    pub const ITERATOR_ITERATOR: u16 = 20;
    pub const STRING_ITERATOR_NEXT: u16 = 21;
    // ---- M2-P0: stubs until their owners fill them (generator.rs, P1-GEN-VM; eval, P3) ----
    /// `%GeneratorFunction%`.
    pub const GENERATOR_FUNCTION: u16 = 22;
    /// `%GeneratorPrototype%.next`.
    pub const GENERATOR_NEXT: u16 = 23;
    /// `%GeneratorPrototype%.return`.
    pub const GENERATOR_RETURN: u16 = 24;
    /// `%GeneratorPrototype%.throw`.
    pub const GENERATOR_THROW: u16 = 25;
    /// `%AsyncFunction%` (month 3).
    pub const ASYNC_FUNCTION: u16 = 26;
    /// `%eval%`, also the global `eval` (P3-EVAL-VM).
    pub const EVAL: u16 = 27;
    // ---- M2-P4 (B-ARRAY) ----
    /// `Array.prototype.keys`.
    pub const ARRAY_KEYS: u16 = 28;
    /// `Array.prototype.entries`.
    pub const ARRAY_ENTRIES: u16 = 29;
    // ---- M2-P5 (VM-SMALL) ----
    /// `Function.prototype[@@hasInstance]` (`property::instance_of` runs it without a call).
    pub const FUNCTION_HAS_INSTANCE: u16 = 30;
    /// The getter of `%Iterator.prototype%[@@toStringTag]`.
    pub const ITERATOR_TO_STRING_TAG_GET: u16 = 31;
    /// The setter of `%Iterator.prototype%[@@toStringTag]`.
    pub const ITERATOR_TO_STRING_TAG_SET: u16 = 32;
}

/// The VM's block.
pub(crate) const VM_BLOCK: u8 = 0;

const VM_DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("", 0, function_prototype),
    BuiltinDef::constructor("Function", 1, function_constructor),
    BuiltinDef::constructor("Object", 1, object_constructor),
    BuiltinDef::method("toString", 0, object_to_string),
    BuiltinDef::method("valueOf", 0, object_value_of),
    BuiltinDef::constructor("Array", 1, array_constructor),
    BuiltinDef::method("join", 1, array_join),
    BuiltinDef::method("toString", 0, array_to_string),
    BuiltinDef::method("push", 1, array_push),
    BuiltinDef::method("next", 0, iterator_next),
    BuiltinDef::constructor("Error", 1, error_ctor),
    BuiltinDef::constructor("TypeError", 1, type_error_ctor),
    BuiltinDef::constructor("RangeError", 1, range_error_ctor),
    BuiltinDef::constructor("ReferenceError", 1, reference_error_ctor),
    BuiltinDef::constructor("SyntaxError", 1, syntax_error_ctor),
    BuiltinDef::constructor("EvalError", 1, eval_error_ctor),
    BuiltinDef::constructor("URIError", 1, uri_error_ctor),
    BuiltinDef::method("toString", 0, error_to_string),
    BuiltinDef::method("values", 0, array_values),
    BuiltinDef::method("[Symbol.iterator]", 0, string_iterator),
    BuiltinDef::method("[Symbol.iterator]", 0, iterator_iterator),
    BuiltinDef::method("next", 0, string_iterator_next),
    BuiltinDef::constructor("GeneratorFunction", 1, generator::generator_function),
    BuiltinDef::method("next", 1, generator::generator_next),
    BuiltinDef::method("return", 1, generator::generator_return),
    BuiltinDef::method("throw", 1, generator::generator_throw),
    BuiltinDef::constructor("AsyncFunction", 1, generator::async_function),
    BuiltinDef::method("eval", 1, eval),
    BuiltinDef::method("keys", 0, array_keys),
    BuiltinDef::method("entries", 0, array_entries),
    BuiltinDef::method("[Symbol.hasInstance]", 1, function_has_instance),
    BuiltinDef::getter("[Symbol.toStringTag]", iterator_to_string_tag_get),
    BuiltinDef::setter("[Symbol.toStringTag]", iterator_to_string_tag_set),
];

const VM_INTRINSICS: &[IntrinsicDef] = &[
    // Core: `realm::init` makes it from this builtin before any other function.
    IntrinsicDef {
        id: IntrinsicId::FunctionPrototype,
        value: IntrinsicValue::Builtin(vm::FUNCTION_PROTOTYPE),
    },
    IntrinsicDef {
        id: IntrinsicId::Function,
        value: IntrinsicValue::Builtin(vm::FUNCTION),
    },
    IntrinsicDef {
        id: IntrinsicId::Object,
        value: IntrinsicValue::Builtin(vm::OBJECT),
    },
    IntrinsicDef {
        id: IntrinsicId::Array,
        value: IntrinsicValue::Builtin(vm::ARRAY),
    },
    IntrinsicDef {
        id: IntrinsicId::Error,
        value: IntrinsicValue::Builtin(vm::ERROR),
    },
    IntrinsicDef {
        id: IntrinsicId::TypeError,
        value: IntrinsicValue::Builtin(vm::TYPE_ERROR),
    },
    IntrinsicDef {
        id: IntrinsicId::RangeError,
        value: IntrinsicValue::Builtin(vm::RANGE_ERROR),
    },
    IntrinsicDef {
        id: IntrinsicId::ReferenceError,
        value: IntrinsicValue::Builtin(vm::REFERENCE_ERROR),
    },
    IntrinsicDef {
        id: IntrinsicId::SyntaxError,
        value: IntrinsicValue::Builtin(vm::SYNTAX_ERROR),
    },
    IntrinsicDef {
        id: IntrinsicId::EvalError,
        value: IntrinsicValue::Builtin(vm::EVAL_ERROR),
    },
    IntrinsicDef {
        id: IntrinsicId::UriError,
        value: IntrinsicValue::Builtin(vm::URI_ERROR),
    },
    // §22.1.5.2: an ordinary object whose [[Prototype]] is %Iterator.prototype%.
    IntrinsicDef {
        id: IntrinsicId::StringIteratorPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::IteratorPrototype),
        },
    },
    // M2-P0. Their `[[Prototype]]` is %Function% (§27.3.2, §27.7.2): a table gives a builtin
    // %Function.prototype%, so `builtins::install` sets it after step (2) (M2-P1-GENFN-PROTO).
    IntrinsicDef {
        id: IntrinsicId::GeneratorFunction,
        value: IntrinsicValue::Builtin(vm::GENERATOR_FUNCTION),
    },
    // §27.3.3: an ordinary object whose [[Prototype]] is %Function.prototype%.
    IntrinsicDef {
        id: IntrinsicId::GeneratorFunctionPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::FunctionPrototype),
        },
    },
    // §27.5.1: an ordinary object whose [[Prototype]] is %Iterator.prototype%.
    IntrinsicDef {
        id: IntrinsicId::GeneratorPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::IteratorPrototype),
        },
    },
    IntrinsicDef {
        id: IntrinsicId::AsyncFunction,
        value: IntrinsicValue::Builtin(vm::ASYNC_FUNCTION),
    },
    // §27.7.3: an ordinary object whose [[Prototype]] is %Function.prototype%.
    IntrinsicDef {
        id: IntrinsicId::AsyncFunctionPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::FunctionPrototype),
        },
    },
    IntrinsicDef {
        id: IntrinsicId::Eval,
        value: IntrinsicValue::Builtin(vm::EVAL),
    },
    // M2-P5. §23.1.3.41: OrdinaryObjectCreate(null).
    IntrinsicDef {
        id: IntrinsicId::ArrayUnscopables,
        value: IntrinsicValue::Object { proto: None },
    },
];

const PROTOTYPE: PropKey = PropKey::Atom(Atom::Prototype);
const CONSTRUCTOR: PropKey = PropKey::Atom(Atom::Constructor);
const TO_STRING: PropKey = PropKey::Atom(Atom::ToStringKey);
const NEXT: PropKey = PropKey::Atom(Atom::Next);
const VALUES: PropKey = PropKey::Str("values");
const ITERATOR: PropKey = PropKey::Symbol(WellKnownSymbol::ITERATOR);
const TO_STRING_TAG: PropKey = PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG);
const HAS_INSTANCE: PropKey = PropKey::Symbol(WellKnownSymbol::HAS_INSTANCE);
const UNSCOPABLES: PropKey = PropKey::Symbol(WellKnownSymbol::UNSCOPABLES);

/// `key` = true {w, e, c}: one entry of `%Array.prototype%[@@unscopables]` (§23.1.3.41,
/// CreateDataPropertyOrThrow).
const fn unscopable(key: &'static str) -> PropDef {
    PropDef::constant(PropKey::Str(key), ConstValue::Bool(true), PropAttrs::ALL)
}

/// `@@toStringTag` = `tag` ({c}, §18): the tag of `Object.prototype.toString` (step 15).
const fn to_string_tag(tag: &'static str) -> PropDef {
    PropDef::constant(TO_STRING_TAG, ConstValue::Str(tag), PropAttrs::CONFIGURABLE)
}

/// `C.prototype = P` ({}): a builtin constructor's `prototype` (§20.1.2.20 and the others).
const fn prototype_of(p: IntrinsicId) -> [PropDef; 1] {
    [PropDef::intrinsic(PROTOTYPE, p, PropAttrs::NONE)]
}

/// The `name`, `message` and `constructor` of a native error prototype (§20.5.6.3).
const fn native_error_proto(name: &'static str, ctor: u16) -> [PropDef; 3] {
    [
        PropDef::constant(
            PropKey::Atom(Atom::Name),
            ConstValue::Str(name),
            PropAttrs::METHOD,
        ),
        PropDef::constant(
            PropKey::Atom(Atom::Message),
            ConstValue::Str(""),
            PropAttrs::METHOD,
        ),
        PropDef::method(CONSTRUCTOR, ctor),
    ]
}

const VM_PROPS: &[ObjectProps] = &[
    // §19.1 value properties, then the constructors (the week-3 order, then `Function`).
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::GlobalThis),
                IntrinsicId::GlobalObject,
                PropAttrs::METHOD,
            ),
            PropDef::constant(
                PropKey::Atom(Atom::Undefined),
                ConstValue::Undefined,
                PropAttrs::NONE,
            ),
            PropDef::constant(
                PropKey::Atom(Atom::NaN),
                ConstValue::Number(f64::NAN),
                PropAttrs::NONE,
            ),
            PropDef::constant(
                PropKey::Atom(Atom::Infinity),
                ConstValue::Number(f64::INFINITY),
                PropAttrs::NONE,
            ),
            PropDef::method(PropKey::Str("Object"), vm::OBJECT),
            PropDef::method(PropKey::Str("Array"), vm::ARRAY),
            PropDef::method(PropKey::Str("Error"), vm::ERROR),
            PropDef::method(PropKey::Str("TypeError"), vm::TYPE_ERROR),
            PropDef::method(PropKey::Str("RangeError"), vm::RANGE_ERROR),
            PropDef::method(PropKey::Str("ReferenceError"), vm::REFERENCE_ERROR),
            PropDef::method(PropKey::Str("SyntaxError"), vm::SYNTAX_ERROR),
            PropDef::method(PropKey::Str("EvalError"), vm::EVAL_ERROR),
            PropDef::method(PropKey::Str("URIError"), vm::URI_ERROR),
            PropDef::method(PropKey::Str("Function"), vm::FUNCTION),
            // §19.2.1: {w, c} (P3-EVAL-VM; C19 kept it out until the VM ran eval code).
            PropDef::method(PropKey::Str("eval"), vm::EVAL),
        ],
    },
    ObjectProps {
        target: IntrinsicId::ObjectPrototype,
        props: &[
            PropDef::method(TO_STRING, vm::OBJECT_TO_STRING),
            PropDef::method(PropKey::Atom(Atom::ValueOf), vm::OBJECT_VALUE_OF),
            PropDef::method(CONSTRUCTOR, vm::OBJECT),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Object,
        props: &prototype_of(IntrinsicId::ObjectPrototype),
    },
    ObjectProps {
        target: IntrinsicId::FunctionPrototype,
        props: &[
            PropDef::method(CONSTRUCTOR, vm::FUNCTION),
            // §20.2.3.6: {w: false, e: false, c: false}.
            PropDef {
                key: HAS_INSTANCE,
                value: PropValue::Builtin(vm::FUNCTION_HAS_INSTANCE),
                attrs: PropAttrs::NONE,
            },
        ],
    },
    ObjectProps {
        target: IntrinsicId::Function,
        props: &prototype_of(IntrinsicId::FunctionPrototype),
    },
    ObjectProps {
        target: IntrinsicId::ArrayPrototype,
        props: &[
            PropDef::method(PropKey::Str("join"), vm::ARRAY_JOIN),
            PropDef::method(TO_STRING, vm::ARRAY_TO_STRING),
            PropDef::method(PropKey::Str("push"), vm::ARRAY_PUSH),
            PropDef::method(CONSTRUCTOR, vm::ARRAY),
            PropDef::method(VALUES, vm::ARRAY_VALUES),
            // §23.1.3.40: the same function object as `values`.
            PropDef::alias(ITERATOR, VALUES, PropAttrs::METHOD),
            PropDef::method(PropKey::Str("keys"), vm::ARRAY_KEYS),
            PropDef::method(PropKey::Str("entries"), vm::ARRAY_ENTRIES),
            // §23.1.3.41: {w: false, e: false, c: true}.
            PropDef::intrinsic(
                UNSCOPABLES,
                IntrinsicId::ArrayUnscopables,
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    // §23.1.3.41 steps 2-18, in spec (and node) order.
    ObjectProps {
        target: IntrinsicId::ArrayUnscopables,
        props: &[
            unscopable("at"),
            unscopable("copyWithin"),
            unscopable("entries"),
            unscopable("fill"),
            unscopable("find"),
            unscopable("findIndex"),
            unscopable("findLast"),
            unscopable("findLastIndex"),
            unscopable("flat"),
            unscopable("flatMap"),
            unscopable("includes"),
            unscopable("keys"),
            unscopable("toReversed"),
            unscopable("toSorted"),
            unscopable("toSpliced"),
            unscopable("values"),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Array,
        props: &prototype_of(IntrinsicId::ArrayPrototype),
    },
    ObjectProps {
        target: IntrinsicId::IteratorPrototype,
        props: &[
            PropDef::method(ITERATOR, vm::ITERATOR_ITERATOR),
            // §27.1.4.14 (ES2025): an accessor {e: false, c: true}, not a data property.
            PropDef::accessor(
                TO_STRING_TAG,
                Some(vm::ITERATOR_TO_STRING_TAG_GET),
                Some(vm::ITERATOR_TO_STRING_TAG_SET),
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::ArrayIteratorPrototype,
        props: &[
            PropDef::method(NEXT, vm::ITERATOR_NEXT),
            to_string_tag("Array Iterator"),
        ],
    },
    ObjectProps {
        target: IntrinsicId::StringIteratorPrototype,
        props: &[
            PropDef::method(NEXT, vm::STRING_ITERATOR_NEXT),
            to_string_tag("String Iterator"),
        ],
    },
    ObjectProps {
        target: IntrinsicId::StringPrototype,
        props: &[PropDef::method(ITERATOR, vm::STRING_ITERATOR)],
    },
    ObjectProps {
        target: IntrinsicId::ErrorPrototype,
        props: &[
            PropDef::method(TO_STRING, vm::ERROR_TO_STRING),
            PropDef::constant(
                PropKey::Atom(Atom::Name),
                ConstValue::Str("Error"),
                PropAttrs::METHOD,
            ),
            PropDef::constant(
                PropKey::Atom(Atom::Message),
                ConstValue::Str(""),
                PropAttrs::METHOD,
            ),
            PropDef::method(CONSTRUCTOR, vm::ERROR),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Error,
        props: &prototype_of(IntrinsicId::ErrorPrototype),
    },
    ObjectProps {
        target: IntrinsicId::TypeErrorPrototype,
        props: &native_error_proto("TypeError", vm::TYPE_ERROR),
    },
    ObjectProps {
        target: IntrinsicId::TypeError,
        props: &prototype_of(IntrinsicId::TypeErrorPrototype),
    },
    ObjectProps {
        target: IntrinsicId::RangeErrorPrototype,
        props: &native_error_proto("RangeError", vm::RANGE_ERROR),
    },
    ObjectProps {
        target: IntrinsicId::RangeError,
        props: &prototype_of(IntrinsicId::RangeErrorPrototype),
    },
    ObjectProps {
        target: IntrinsicId::ReferenceErrorPrototype,
        props: &native_error_proto("ReferenceError", vm::REFERENCE_ERROR),
    },
    ObjectProps {
        target: IntrinsicId::ReferenceError,
        props: &prototype_of(IntrinsicId::ReferenceErrorPrototype),
    },
    ObjectProps {
        target: IntrinsicId::SyntaxErrorPrototype,
        props: &native_error_proto("SyntaxError", vm::SYNTAX_ERROR),
    },
    ObjectProps {
        target: IntrinsicId::SyntaxError,
        props: &prototype_of(IntrinsicId::SyntaxErrorPrototype),
    },
    ObjectProps {
        target: IntrinsicId::EvalErrorPrototype,
        props: &native_error_proto("EvalError", vm::EVAL_ERROR),
    },
    ObjectProps {
        target: IntrinsicId::EvalError,
        props: &prototype_of(IntrinsicId::EvalErrorPrototype),
    },
    ObjectProps {
        target: IntrinsicId::UriErrorPrototype,
        props: &native_error_proto("URIError", vm::URI_ERROR),
    },
    ObjectProps {
        target: IntrinsicId::UriError,
        props: &prototype_of(IntrinsicId::UriErrorPrototype),
    },
    // ---- M2-P0: generators (§27.3, §27.5) and async functions (§27.7) ----
    ObjectProps {
        target: IntrinsicId::GeneratorFunction,
        props: &prototype_of(IntrinsicId::GeneratorFunctionPrototype),
    },
    ObjectProps {
        target: IntrinsicId::GeneratorFunctionPrototype,
        props: &[
            // §27.3.3.1, §27.3.3.2: {c}.
            PropDef {
                key: CONSTRUCTOR,
                value: PropValue::Builtin(vm::GENERATOR_FUNCTION),
                attrs: PropAttrs::CONFIGURABLE,
            },
            PropDef::intrinsic(
                PROTOTYPE,
                IntrinsicId::GeneratorPrototype,
                PropAttrs::CONFIGURABLE,
            ),
            to_string_tag("GeneratorFunction"),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GeneratorPrototype,
        props: &[
            // §27.5.1.1: {c}.
            PropDef::intrinsic(
                CONSTRUCTOR,
                IntrinsicId::GeneratorFunctionPrototype,
                PropAttrs::CONFIGURABLE,
            ),
            PropDef::method(NEXT, vm::GENERATOR_NEXT),
            PropDef::method(PropKey::Str("return"), vm::GENERATOR_RETURN),
            PropDef::method(PropKey::Str("throw"), vm::GENERATOR_THROW),
            to_string_tag("Generator"),
        ],
    },
    ObjectProps {
        target: IntrinsicId::AsyncFunction,
        props: &prototype_of(IntrinsicId::AsyncFunctionPrototype),
    },
    ObjectProps {
        target: IntrinsicId::AsyncFunctionPrototype,
        props: &[
            // §27.7.3.1: {c}.
            PropDef {
                key: CONSTRUCTOR,
                value: PropValue::Builtin(vm::ASYNC_FUNCTION),
                attrs: PropAttrs::CONFIGURABLE,
            },
            to_string_tag("AsyncFunction"),
        ],
    },
];

/// The VM module: block 0.
pub(crate) const VM_MODULE: BuiltinModule = BuiltinModule {
    name: "vm",
    block: VM_BLOCK,
    defs: VM_DEFS,
    intrinsics: VM_INTRINSICS,
    props: VM_PROPS,
};

/// The VM's own table; realm init walks it before `IsolateConfig::builtins`.
pub(crate) static VM_TABLE: BuiltinTable = BuiltinTable::new("vm", &[VM_MODULE]);

// ---- functions ---------------------------------------------------------------------------------

/// A `Local` of VM bits.
fn local<'s>(b: u64) -> Local<'s> {
    Local::from_bits(b)
}

/// `%Function.prototype%` (§20.2.3): accepts any arguments and returns undefined.
pub(crate) fn function_prototype<'s>(
    _cx: &mut HostCx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::undefined())
}

/// `Function(...params, body)` (§20.2.1.1): CreateDynamicFunction with kind normal, through
/// the compiler hook (`abi/script.rs`; an EvalError on an isolate without hooks, O2).
pub(crate) fn function_constructor<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.create_dynamic_function(args, nt, ep9_bytecode::FunctionCtorKind::Normal)
}

/// `eval(x)` (§19.2.1), `%eval%`: indirect eval (PerformEval with `direct` false). A direct
/// `eval(...)` call does not reach this function with a string: `ResolvePossiblyDirectEval`
/// (`interp/slow/eval.rs`) runs the code in the caller's scope instead.
pub(crate) fn eval<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.indirect_eval(args.get(0))
}

/// Object(value) (§20.1.1.1).
pub(crate) fn object_constructor<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is neither undefined nor the active function object, return
    //    ? OrdinaryCreateFromConstructor(NewTarget, "%Object.prototype%").
    if let Some(nt) = nt.as_object()
        && !nt.is_same_bits(&args.callee())
    {
        let o = cx.ordinary_create_from_constructor(nt, IntrinsicId::ObjectPrototype)?;
        return Ok(o.value());
    }
    // 2. If value is either undefined or null, return OrdinaryObjectCreate(%Object.prototype%).
    let v = args.get(0);
    if v.is_undefined_or_null() {
        return Ok(cx.new_object().value());
    }
    // 3. Return ! ToObject(value).
    Ok(cx.to_object(v)?.value())
}

/// Array(...items) and Array(length) (§23.1.1.1). A NewTarget other than `Array` itself (a
/// subclass `super(...)`, `Reflect.construct`) gives the prototype through
/// GetPrototypeFromConstructor(NewTarget, %Array.prototype%) (step 2), before the length
/// check; `Array` itself or no NewTarget keeps %Array.prototype% (its `prototype` is not
/// writable or configurable).
pub(crate) fn array_constructor<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 2. Let proto be ? GetPrototypeFromConstructor(newTarget, "%Array.prototype%"). GC point
    //    (a getter may run): `proto` then lives in a Rust local.
    let proto = match nt.as_object() {
        Some(nt) if !nt.is_same_bits(&args.callee()) => Some(
            cx.get_prototype_from_constructor(nt, IntrinsicId::ArrayPrototype)?
                .bits(),
        ),
        _ => None,
    };
    let rt = cx.inner().rt();
    // ArrayCreate(len, proto) (§10.4.2.2) step 3: the [[Prototype]] of a new array. Not a
    // cycle: the array is new. GC point: `a` and `proto` live in Rust locals.
    let with_proto = |a: u64| {
        if let Some(p) = proto
            && p != rt.root(Root::ArrayPrototype)
        {
            object::set_proto(rt, a, p);
        }
        a
    };
    if args.len() == 1
        && let Some(n) = args.get(0).as_number()
    {
        let len = n as u32;
        if f64::from(len) != n {
            return Err(cx.throw_error(ErrorKind::RangeError, "Invalid array length"));
        }
        let a = with_proto(object::new_array(rt, 0));
        if !object::array_set_length(rt, a, len) {
            return Err(cx.throw_error(ErrorKind::RangeError, "Invalid array length"));
        }
        return Ok(local(a));
    }
    let a = with_proto(object::new_array(rt, args.len()));
    for (i, v) in args.iter().enumerate() {
        object::array_set(rt, a, i as u32, v.bits());
    }
    Ok(local(a))
}

/// The `Error` constructor and the native error constructors (§20.5.1.1, §20.5.6.1.1).
macro_rules! error_ctors {
    ($($f:ident => $kind:ident,)*) => {$(
        pub(crate) fn $f<'s>(
            cx: &mut HostCx<'s>,
            _this: Local<'s>,
            args: Args<'s>,
            new_target: Local<'s>,
        ) -> JsResult<Local<'s>> {
            let (m, nt) = (args.get(0).bits(), new_target.bits());
            construct_error(cx.inner(), ErrorKind::$kind, m, nt).map(local)
        }
    )*};
}

error_ctors! {
    error_ctor => Error,
    type_error_ctor => TypeError,
    range_error_ctor => RangeError,
    reference_error_ctor => ReferenceError,
    syntax_error_ctor => SyntaxError,
    eval_error_ctor => EvalError,
    uri_error_ctor => UriError,
}

/// NativeError(message) (§20.5.6.1.1). v0: no `options.cause`.
fn construct_error(cx: &mut Cx, kind: ErrorKind, message: u64, nt: u64) -> JsResult<u64> {
    let rt = cx.rt();
    let mut proto = rt.root(kind.prototype_root());
    if is_callable(nt) {
        let p = property::get(cx, nt, rt.atom(Atom::Prototype))?;
        if is_object(p) {
            proto = p;
        }
    }
    let msg = if message == UNDEFINED {
        None
    } else {
        Some(conv::to_string(cx, message)?)
    };
    Ok(error::new_error_value(cx, proto, msg))
}

// ---- iterators ---------------------------------------------------------------------------------

/// A new array iterator over the object `o` (CreateArrayIterator, §23.1.5.1, kind "value").
/// GC point: `o` lives in a Rust local.
pub(crate) fn new_array_iterator(rt: &Runtime, o: u64) -> u64 {
    debug_assert!(is_object(o));
    object::new_iterator(rt, rt.root(Root::ArrayIteratorPrototype), o)
}

/// [`new_array_iterator`] with the kind `k` (`object::iter_kind`: key, value or
/// key+value). GC point: `o` lives in a Rust local.
fn new_array_iterator_of_kind(rt: &Runtime, o: u64, k: i32) -> u64 {
    debug_assert!(is_object(o));
    object::new_iterator_of_kind(rt, rt.root(Root::ArrayIteratorPrototype), o, k)
}

/// A new string iterator over the string `s` (CreateStringIterator, §22.1.5.1). GC point.
pub(crate) fn new_string_iterator(rt: &Runtime, s: u64) -> u64 {
    debug_assert!(is_string(s));
    let proto = builtins::intrinsic(rt, IntrinsicId::StringIteratorPrototype);
    object::new_iterator(rt, proto, s)
}

/// Array.prototype.values (§23.1.3.38), which is also `Array.prototype[@@iterator]`
/// (§23.1.3.40): CreateArrayIterator(? ToObject(this value), value).
pub(crate) fn array_values<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    _: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    let o = cx.to_object(this)?.bits();
    Ok(local(new_array_iterator(cx.inner().rt(), o)))
}

/// Array.prototype.keys (§23.1.3.19): CreateArrayIterator(? ToObject(this value), key).
pub(crate) fn array_keys<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    _: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    let o = cx.to_object(this)?.bits();
    let it = new_array_iterator_of_kind(cx.inner().rt(), o, object::iter_kind::KEYS);
    Ok(local(it))
}

/// Array.prototype.entries (§23.1.3.5): CreateArrayIterator(? ToObject(this value),
/// key+value).
pub(crate) fn array_entries<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    _: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    let o = cx.to_object(this)?.bits();
    let it = new_array_iterator_of_kind(cx.inner().rt(), o, object::iter_kind::ENTRIES);
    Ok(local(it))
}

/// `String.prototype[@@iterator]` (§22.1.3.36): CreateStringIterator(? ToString(?
/// RequireObjectCoercible(this value))).
pub(crate) fn string_iterator<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    _: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    if this.is_undefined_or_null() {
        return Err(
            cx.throw_type_error("String.prototype[Symbol.iterator] called on null or undefined")
        );
    }
    let s = cx.to_string(this)?.bits();
    Ok(local(new_string_iterator(cx.inner().rt(), s)))
}

/// `%Iterator.prototype%[@@iterator]` (§27.1.2.1): the this value.
pub(crate) fn iterator_iterator<'s>(
    _cx: &mut HostCx<'s>,
    this: Local<'s>,
    _: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(this)
}

/// The getter of `%Iterator.prototype%[@@toStringTag]` (§27.1.4.14.1, ES2025): "Iterator".
pub(crate) fn iterator_to_string_tag_get<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    _: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(cx.new_string("Iterator")?.value())
}

/// The setter of `%Iterator.prototype%[@@toStringTag]` (§27.1.4.14.2, ES2025):
/// SetterThatIgnoresPrototypeProperties(this value, %Iterator.prototype%, @@toStringTag, v)
/// (§27.1.3.2.4), then undefined.
pub(crate) fn iterator_to_string_tag_set<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    let cx = cx.inner();
    let (o, v) = (this.bits(), args.get(0).bits());
    // 1. If this is not an Object, throw a TypeError exception.
    if !is_object(o) {
        return Err(error::throw_error(
            cx,
            ErrorKind::TypeError,
            "set Iterator.prototype[Symbol.toStringTag] called on non-object",
        ));
    }
    let rt = cx.rt();
    let key = symbol::well_known(rt, symbol::WellKnownSymbol::ToStringTag);
    // 2. If SameValue(this, home) is true, throw a TypeError exception (the strict-mode
    //    write to a non-writable data property on home).
    if o == builtins::intrinsic(rt, IntrinsicId::IteratorPrototype) {
        return Err(error::throw_error(
            cx,
            ErrorKind::TypeError,
            "Cannot assign to read only property 'Symbol(Symbol.toStringTag)' of object",
        ));
    }
    // 3. Let desc be ? this.[[GetOwnProperty]](p). GC point below: `o`, `key` and `v` live in
    //    Rust locals.
    if property::get_own_property(cx, o, key)?.is_none() {
        // 4. If desc is undefined: ? CreateDataPropertyOrThrow(this, p, v).
        let desc = crate::descriptor::Desc::data(v, attr::DEFAULT);
        if !property::define_own_property(cx, o, key, desc)? {
            return Err(error::throw_error(
                cx,
                ErrorKind::TypeError,
                "Cannot define property Symbol(Symbol.toStringTag), object is not extensible",
            ));
        }
    } else {
        // 5. Else: ? Set(this, p, v, true).
        property::set(cx, o, key, v, true)?;
    }
    Ok(Local::undefined())
}

/// `Function.prototype[@@hasInstance](V)` (§20.2.3.6): OrdinaryHasInstance(this value, V).
/// `instanceof` does not call it: `property::instance_of` runs the same steps when it finds
/// this function.
pub(crate) fn function_has_instance<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    let (f, v) = (this.bits(), args.get(0).bits());
    let r = property::ordinary_has_instance(cx.inner(), f, v)?;
    Ok(local(if r { TRUE } else { FALSE }))
}

/// The TypeError of a `next` whose receiver is not an iterator of its kind
/// (GeneratorValidate, §27.5.3.2).
fn incompatible_receiver(cx: &mut Cx, what: &str, this: u64) -> ep9_value::Throw {
    let msg = format!(
        "Method {what}.prototype.next called on incompatible receiver {}",
        conv::describe(this)
    );
    error::throw_error(cx, ErrorKind::TypeError, &msg)
}

/// CreateIteratorResultObject(value, done) (§7.4.14). GC point: `value` lives in a Rust local.
fn iter_result(rt: &Runtime, value: u64, done: bool) -> u64 {
    let result = object::new_plain_object(rt);
    own_define(rt, result, rt.atom(Atom::ValueKey), value, attr::DEFAULT);
    let done = if done { TRUE } else { FALSE };
    own_define(rt, result, rt.atom(Atom::Done), done, attr::DEFAULT);
    result
}

/// %ArrayIteratorPrototype%.next (§23.1.5.2.1): one step of the CreateArrayIterator closure
/// (§23.1.5.1) of the cell's kind: the index (key), the element (value) or `[index, element]`
/// (key+value). The length is read at every step (`LengthOfArrayLike`). Once the index
/// reaches it, or a step throws (the closure completes), the iterator is done for good. While
/// the step runs (getters, `valueOf` of the length), the cell holds null (executing).
pub(crate) fn iterator_next<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    _: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    let cx = cx.inner();
    let it = this.bits();
    // GeneratorResume (§27.5.3.3) step 1: GeneratorValidate (§27.5.3.2), the brand first.
    let state = object::is_iterator(it)
        .then(|| object::iterator_state(it))
        .filter(|&(target, _)| !is_string(target));
    let Some((target, i)) = state else {
        return Err(incompatible_receiver(cx, "Array Iterator", it));
    };
    // GeneratorValidate step 6: executing, so the call comes from the user code of a step
    // (V8 runs it instead: W4-P3FIX-ITER-REENTRY).
    if target == NULL {
        let msg = "Array Iterator is already running";
        return Err(error::throw_error(cx, ErrorKind::TypeError, msg));
    }
    let rt = cx.rt();
    // GeneratorResume step 2: completed.
    if target == UNDEFINED {
        return Ok(local(iter_result(rt, UNDEFINED, true)));
    }
    // GeneratorResume step 7: executing until the step ends. GC points below: `target` is
    // only in a Rust local meanwhile.
    let kind = object::iterator_kind(it);
    object::set_iterator_state(rt, it, NULL, i);
    match array_iterator_step(cx, target, i, kind) {
        Ok(Some(value)) => {
            object::set_iterator_state(rt, it, target, i + 1);
            Ok(local(iter_result(rt, value, false)))
        }
        Ok(None) => {
            object::set_iterator_state(rt, it, UNDEFINED, i);
            Ok(local(iter_result(rt, UNDEFINED, true)))
        }
        Err(e) => {
            object::set_iterator_state(rt, it, UNDEFINED, i);
            Err(e)
        }
    }
}

/// The result of step `i` of an array iterator of kind `kind` over the array-like `o`
/// (§23.1.5.1 steps 2.b.iii-v): the index `i` (key), element `i` (value) or a new Array
/// `[i, element]` (key+value, CreateArrayFromList); `None` past `LengthOfArrayLike(o)`
/// (§7.3.18). A key step reads no element. v0: the cell keeps a u32 index, so an array-like
/// is done at 2^32 - 1 (an Array's length is below that). GC point.
fn array_iterator_step(cx: &mut Cx, o: u64, i: u32, kind: i32) -> JsResult<Option<u64>> {
    let len = if is_array(o) {
        f64::from(object::array_len(o))
    } else {
        let rt = cx.rt();
        let len = property::get(cx, o, rt.atom(Atom::Length))?;
        to_length(cx, len)?
    };
    if f64::from(i) >= len || i == u32::MAX {
        return Ok(None);
    }
    let index = num(f64::from(i));
    // iii. If kind is key, then let result be indexNumber.
    if kind == object::iter_kind::KEYS {
        return Ok(Some(index));
    }
    // iv. Else, let elementValue be ? Get(array, ! ToString(indexNumber)).
    let element = property::get_keyed(cx, o, index)?;
    // iv.2. If kind is value, then let result be elementValue.
    if kind == object::iter_kind::VALUES {
        return Ok(Some(element));
    }
    // iv.3. Else, kind is key+value: let result be CreateArrayFromList(« indexNumber,
    //       elementValue »). GC point: `element` lives in a Rust local.
    debug_assert_eq!(kind, object::iter_kind::ENTRIES);
    let rt = cx.rt();
    let pair = object::new_array(rt, 2);
    object::array_set(rt, pair, 0, index);
    object::array_set(rt, pair, 1, element);
    Ok(Some(pair))
}

/// ToLength (§7.1.20) as an f64 in [0, 2^53 - 1].
fn to_length(cx: &mut Cx, v: u64) -> JsResult<f64> {
    let d = conv::to_number(cx, v)?;
    Ok(if d.is_nan() || d <= 0.0 {
        0.0
    } else {
        d.trunc().min(9_007_199_254_740_991.0)
    })
}

/// %StringIteratorPrototype%.next (§22.1.5.2.1): one step of the CreateStringIterator closure
/// (§22.1.5.1). A done iterator holds the empty string, so it keeps its brand.
pub(crate) fn string_iterator_next<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    _: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    let cx = cx.inner();
    let it = this.bits();
    let state = object::is_iterator(it)
        .then(|| object::iterator_state(it))
        .filter(|&(s, _)| is_string(s));
    let Some((s, i)) = state else {
        return Err(incompatible_receiver(cx, "String Iterator", it));
    };
    let rt = cx.rt();
    let (value, done) = match code_point_string(cx, s, i as usize) {
        Some((cp, len)) => {
            object::set_iterator_state(rt, it, s, i + len as u32);
            (cp, false)
        }
        None => {
            object::set_iterator_state(rt, it, rt.atom(Atom::Empty), 0);
            (UNDEFINED, true)
        }
    };
    Ok(local(iter_result(rt, value, done)))
}

/// The code point of the string `s` at code unit `i` (CodePointAt, §11.1.4) as a string,
/// and its length in code units: one unit gives `string::char_at` (the single-character atom
/// for a Latin-1 unit), a surrogate pair a new string. `None` at or past the end. GC point.
pub(crate) fn code_point_string(cx: &mut Cx, s: u64, i: usize) -> Option<(u64, usize)> {
    let s = string::flatten(cx, s);
    // The view ends before the allocation below (W4-STRVIEW).
    let pair = {
        let v = string::view(cx, s);
        if i >= v.len() {
            return None;
        }
        let pair = i + 1 < v.len()
            && (0xD800..0xDC00).contains(&v.at(i))
            && (0xDC00..0xE000).contains(&v.at(i + 1));
        pair.then(|| [v.at(i), v.at(i + 1)])
    };
    let rt = cx.rt();
    Some(match pair {
        Some(units) => (string::new_string_utf16(rt, &units), 2),
        None => (string::char_at(rt, s, i), 1),
    })
}

/// Error.prototype.toString (§20.5.3.4).
pub(crate) fn error_to_string<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    _: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    let cx = cx.inner();
    let o = this.bits();
    if !is_object(o) {
        return Err(error::throw_error(
            cx,
            ErrorKind::TypeError,
            "Error.prototype.toString called on a non-object",
        ));
    }
    let rt = cx.rt();
    let name = property::get(cx, o, rt.atom(Atom::Name))?;
    let name = if name == UNDEFINED {
        "Error".to_string()
    } else {
        conv::to_rust_string(cx, name)?
    };
    let msg = property::get(cx, o, rt.atom(Atom::Message))?;
    let msg = if msg == UNDEFINED {
        String::new()
    } else {
        conv::to_rust_string(cx, msg)?
    };
    let s = match (name.is_empty(), msg.is_empty()) {
        (true, _) => msg,
        (_, true) => name,
        _ => format!("{name}: {msg}"),
    };
    Ok(local(object::new_string(rt, &s)))
}

/// Object.prototype.toString (§20.1.3.6). v0: no `[[ParameterMap]]` (arguments objects are
/// Arrays), and no Date or RegExp.
pub(crate) fn object_to_string<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    _: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2.
    if this.is_undefined() {
        return Ok(cx.new_string("[object Undefined]")?.value());
    }
    if this.is_null() {
        return Ok(cx.new_string("[object Null]")?.value());
    }
    // 3. O = ! ToObject(this value). GC point: `o` then lives in a Rust local.
    let o = cx.to_object(this)?.bits();
    // 4-14. builtinTag: a primitive and its wrapper have the same tag, from [[BooleanData]],
    //    [[NumberData]] or [[StringData]].
    let data = local(wrapper::wrapper_value(o).unwrap_or(o));
    let builtin_tag = if is_array(o) {
        "Array"
    } else if is_callable(o) {
        "Function"
    } else if object::is_error(o) {
        "Error"
    } else if data.is_bool() {
        "Boolean"
    } else if data.is_number() {
        "Number"
    } else if data.is_string() {
        "String"
    } else if cx.object_class(local(o)) == Some(crate::abi::ObjectClass::Date) {
        // 13. Else if O has a [[DateValue]] internal slot, let builtinTag be "Date".
        "Date"
    } else {
        "Object"
    };
    // 15. tag = ? Get(O, @@toStringTag).
    let cx = cx.inner();
    let rt = cx.rt();
    let key = symbol::well_known(rt, symbol::WellKnownSymbol::ToStringTag);
    let tag = property::get(cx, o, key)?;
    // 16-17. "[object " + tag + "]", with builtinTag unless tag is a String.
    if !is_string(tag) {
        return Ok(local(string::new_string(
            rt,
            &format!("[object {builtin_tag}]"),
        )));
    }
    let tag = string::flatten(cx, tag);
    if string::str_len(tag) > MAX_STRING_LENGTH - 9 {
        return Err(error::throw_error(
            cx,
            ErrorKind::RangeError,
            "Invalid string length",
        ));
    }
    let mut units: Vec<u16> = "[object ".encode_utf16().collect();
    units.extend(string::view(cx, tag).to_utf16());
    units.push(u16::from(b']'));
    Ok(local(string::new_string_utf16(rt, &units)))
}

/// Object.prototype.valueOf (§20.1.3.7): ? ToObject(this value), so a primitive gets its
/// wrapper object.
pub(crate) fn object_value_of<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    _: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(cx.to_object(this)?.value())
}

/// Array.prototype.join (§23.1.3.18): step 1 is ? ToObject(this value).
pub(crate) fn array_join<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    // GC point (a wrapper): `o` then lives in a Rust local, the separator in the arguments.
    let o = cx.to_object(this)?.bits();
    join(cx.inner(), o, args.get(0).bits()).map(local)
}

/// Array.prototype.join steps 2-8 on the object `o` (step 1 made it).
fn join(cx: &mut Cx, o: u64, sep: u64) -> JsResult<u64> {
    debug_assert!(is_object(o));
    let rt = cx.rt();
    let len = property::get(cx, o, rt.atom(Atom::Length))?;
    let len = conv::to_number(cx, len)?;
    let len = if len.is_nan() || len <= 0.0 {
        0u32
    } else {
        len.min(f64::from(u32::MAX)) as u32
    };
    let sep = if sep == UNDEFINED {
        rt.atom(Atom::Comma)
    } else {
        conv::to_string(cx, sep)?
    };
    let mut acc = rt.atom(Atom::Empty);
    for i in 0..len {
        if i > 0 {
            acc = concat_or_throw(cx, acc, sep)?;
        }
        let e = property::get_keyed(cx, o, num(f64::from(i)))?;
        if e != UNDEFINED && e != NULL {
            let s = conv::to_string(cx, e)?;
            acc = concat_or_throw(cx, acc, s)?;
        }
    }
    Ok(acc)
}

fn concat_or_throw(cx: &mut Cx, a: u64, b: u64) -> JsResult<u64> {
    object::concat(cx.rt(), a, b)
        .ok_or_else(|| error::throw_error(cx, ErrorKind::RangeError, "Invalid string length"))
}

/// Array.prototype.toString (§23.1.3.36): ? ToObject(this value), then Call(? Get(array,
/// "join"), array) if that is callable, else %Object.prototype.toString% on the array. The
/// intrinsic `join` runs without a call.
pub(crate) fn array_to_string<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    a: Args<'s>,
    nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let array be ? ToObject(this value). GC point: `array` then lives in a Rust local.
    let array = cx.to_object(this)?.bits();
    let cx_in = cx.inner();
    // 2. Let func be ? Get(array, "join").
    let key = crate::atoms::intern_str(cx_in.rt(), "join");
    let func = property::get(cx_in, array, key)?;
    // 3. If IsCallable(func) is false, set func to %Object.prototype.toString%.
    if !is_callable(func) {
        return object_to_string(cx, local(array), a, nt);
    }
    // 4. Return ? Call(func, array).
    let id = builtins::BuiltinId::new(VM_BLOCK, vm::ARRAY_JOIN);
    if builtins::existing_function(cx_in.rt(), id) == Some(func) {
        return join(cx_in, array, UNDEFINED).map(local);
    }
    crate::call::call_function(cx_in, func, array, &[]).map(local)
}

/// Array.prototype.push ( ...items ) (§23.1.3.23). An Array takes the fast loop below (or
/// [`push_with_set`] while a prototype has an index key); any other value takes the spec
/// steps through `[[Set]]` ([`push_generic`]).
pub(crate) fn array_push<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    let Some(a) = this.as_array() else {
        return push_generic(cx, this, args);
    };
    let a = a.bits();
    if crate::shape::proto_index_keys(cx.inner().rt()) {
        // A prototype may have an index key: each store is a `[[Set]]` walk.
        let len = f64::from(object::array_len(a));
        return push_with_set(cx.inner(), a, len, args).map(local);
    }
    for v in args.iter() {
        let n = object::array_len(a);
        // Set(O, n, v, true): a non-extensible array rejects the new element.
        if !property::is_extensible(a) {
            let msg = format!("Cannot add property {n}, object is not extensible");
            return Err(cx.throw_type_error(&msg));
        }
        if !object::array_set(cx.inner().rt(), a, n, v.bits()) {
            return Err(cx.throw_error(ErrorKind::RangeError, "Invalid array length"));
        }
    }
    Ok(local(num(f64::from(object::array_len(a)))))
}

/// Array.prototype.push on a value that is not an Array (§23.1.3.23 steps 1-2, then
/// [`push_with_set`]): ToObject, then `LengthOfArrayLike`, which may run a getter or
/// `valueOf`. Out of line, so the fast loop of [`array_push`] stays small.
#[cold]
#[inline(never)]
fn push_generic<'s>(cx: &mut HostCx<'s>, this: Local<'s>, args: Args<'s>) -> JsResult<Local<'s>> {
    // 1. Let O be ? ToObject(this value).
    let o = cx.to_object(this)?.bits();
    let cx = cx.inner();
    // 2. Let len be ? LengthOfArrayLike(O). GC point: `o` lives in a Rust local.
    let key = cx.rt().atom(Atom::Length);
    let len = property::get(cx, o, key)?;
    let len = to_length(cx, len)?;
    push_with_set(cx, o, len, args).map(local)
}

/// Array.prototype.push steps 3-7 on the object `o` of length `len` (step 2) with `[[Set]]`
/// for every store, so a setter, a getter or a read-only index property decides (an Array
/// while `shape::proto_index_keys` holds, or any other object). Out of line, so the fast
/// loop of [`array_push`] stays small.
#[cold]
#[inline(never)]
fn push_with_set(cx: &mut Cx, o: u64, mut len: f64, args: Args<'_>) -> JsResult<u64> {
    // 3. Let argCount be the number of elements in items.
    let arg_count = args.len() as f64;
    // 4. If len + argCount > 2^53 - 1, throw a TypeError exception.
    if len + arg_count > 9_007_199_254_740_991.0 {
        return Err(error::throw_error(
            cx,
            ErrorKind::TypeError,
            "Pushing the items would make the length exceed 2^53 - 1",
        ));
    }
    // 5. For each element E of items: Set(O, ! ToString(𝔽(len)), E, true); len = len + 1.
    //    (The loop is bounded by the argument count; each Set may run JS, which polls.)
    for v in args.iter() {
        // GC point: `o` lives in a Rust local, `v` in the argument list.
        let key = crate::key::number_key(cx.rt(), len);
        property::set(cx, o, key, v.bits(), true)?;
        len += 1.0;
    }
    // 6. Perform ? Set(O, "length", 𝔽(len), true).
    let key = cx.rt().atom(Atom::Length);
    property::set(cx, o, key, num(len), true)?;
    // 7. Return 𝔽(len).
    Ok(num(len))
}
