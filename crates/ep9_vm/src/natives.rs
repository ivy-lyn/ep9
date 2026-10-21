//! The host functions of the VM's own builtin table (`VM_TABLE`, block 0), on the builtin ABI
//! v1 (H20). They live in ep9_vm because they read VM internals (iterator cells, error sites)
//! or are needed before any table: `%Function.prototype%` is the prototype of every function.
//! The other builtins are in `ep9_builtins`.
//!
//! | Function | Spec |
//! |---|---|
//! | `%Function.prototype%` (callable, returns undefined), `Function` (v0: throws) | §20.2.3, §20.2.1.1 |
//! | `Object`, `Array` | §20.1.1.1, §23.1.1.1 |
//! | `Error` and the native errors | §20.5.1.1, §20.5.6.1.1 |
//! | `Object.prototype.toString`, `valueOf` | §20.1.3.6, §20.1.3.7 |
//! | `Error.prototype.toString` | §20.5.3.4 |
//! | `Array.prototype.join`, `toString`, `push` | §23.1.3.18, §23.1.3.36, §23.1.3.23 |
//! | `Array.prototype.values` = `Array.prototype[@@iterator]`, %ArrayIteratorPrototype%.next | §23.1.3.38, §23.1.3.40, §23.1.5.2.1 |
//! | `String.prototype[@@iterator]`, %StringIteratorPrototype%.next | §22.1.3.36, §22.1.5.2.1 |
//! | %Iterator.prototype%[@@iterator] | §27.1.2.1 |
//!
//! The built-in iterators are ITERATOR cells (object.rs): the iterated value and the next
//! index. The iterated value also gives the kind, so each `next` checks its receiver
//! (GeneratorValidate's brand) without a kind field:
//!
//! | Iterator | Iterated value | Done | Executing |
//! |---|---|---|---|
//! | array (`CreateArrayIterator`, kind "value") | an object: an Array, or any array-like (`LengthOfArrayLike`) | undefined | null, while `next` runs user code (getters, `valueOf`) |
//! | string (`CreateStringIterator`) | a string primitive | the empty string | never: its `next` runs no user code |
//!
//! A `next` on an executing iterator throws a TypeError (GeneratorValidate, §27.5.3.2).
//!
//! `interp/slow/iter.rs` (GetIterator, spread) makes the same cells without calling
//! `values` or `[@@iterator]` while the method it finds is the intrinsic.

use ep9_value::JsResult;

use crate::abi::{Args, Cx as HostCx, Local};
use crate::atoms::Atom;
use crate::builtins::{
    self, BuiltinDef, BuiltinModule, BuiltinTable, ConstValue, IntrinsicDef, IntrinsicId,
    IntrinsicValue, ObjectProps, PropAttrs, PropDef, PropKey, WellKnownSymbol,
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
use crate::{symbol, wrapper};

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
];

const PROTOTYPE: PropKey = PropKey::Atom(Atom::Prototype);
const CONSTRUCTOR: PropKey = PropKey::Atom(Atom::Constructor);
const TO_STRING: PropKey = PropKey::Atom(Atom::ToStringKey);
const NEXT: PropKey = PropKey::Atom(Atom::Next);
const VALUES: PropKey = PropKey::Str("values");
const ITERATOR: PropKey = PropKey::Symbol(WellKnownSymbol::ITERATOR);
const TO_STRING_TAG: PropKey = PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG);

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
        props: &[PropDef::method(CONSTRUCTOR, vm::FUNCTION)],
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
        ],
    },
    ObjectProps {
        target: IntrinsicId::Array,
        props: &prototype_of(IntrinsicId::ArrayPrototype),
    },
    ObjectProps {
        target: IntrinsicId::IteratorPrototype,
        props: &[PropDef::method(ITERATOR, vm::ITERATOR_ITERATOR)],
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

/// `Function(...params, body)` (§20.2.1.1). v0: dynamic functions need the compiler in the
/// isolate (H5), so it throws.
pub(crate) fn function_constructor<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.not_implemented("Function constructor"))
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

/// Array(...items) and Array(length) (§23.1.1.1).
pub(crate) fn array_constructor<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let rt = cx.inner().rt();
    if args.len() == 1
        && let Some(n) = args.get(0).as_number()
    {
        let len = n as u32;
        if f64::from(len) != n {
            return Err(cx.throw_error(ErrorKind::RangeError, "Invalid array length"));
        }
        let a = object::new_array(rt, 0);
        if !object::array_set_length(rt, a, len) {
            return Err(cx.throw_error(ErrorKind::RangeError, "Invalid array length"));
        }
        return Ok(local(a));
    }
    let a = object::new_array(rt, args.len());
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

/// String.prototype[@@iterator] (§22.1.3.36): CreateStringIterator(? ToString(?
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

/// %Iterator.prototype%[@@iterator] (§27.1.2.1): the this value.
pub(crate) fn iterator_iterator<'s>(
    _cx: &mut HostCx<'s>,
    this: Local<'s>,
    _: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(this)
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
/// (§23.1.5.1, kind "value"). The length is read at every step (`LengthOfArrayLike`). Once the
/// index reaches it, or a step throws (the closure completes), the iterator is done for good.
/// While the step runs (getters, `valueOf` of the length), the cell holds null (executing).
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
    object::set_iterator_state(rt, it, NULL, i);
    match array_iterator_step(cx, target, i) {
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

/// Element `i` of the array-like `o`, or `None` past `LengthOfArrayLike(o)` (§7.3.18). v0:
/// the cell keeps a u32 index, so an array-like is done at 2^32 - 1 (an Array's length is
/// below that).
fn array_iterator_step(cx: &mut Cx, o: u64, i: u32) -> JsResult<Option<u64>> {
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
    property::get_keyed(cx, o, num(f64::from(i))).map(Some)
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

/// Object.prototype.valueOf (§20.1.3.7, v0: primitives are not wrapped).
pub(crate) fn object_value_of<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    _: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    if this.is_undefined_or_null() {
        return Err(cx.throw_error(
            ErrorKind::TypeError,
            "Cannot convert undefined or null to object",
        ));
    }
    Ok(this)
}

/// Array.prototype.join (§23.1.3.18).
pub(crate) fn array_join<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    join(cx.inner(), this.bits(), args.get(0).bits()).map(local)
}

fn join(cx: &mut Cx, o: u64, sep: u64) -> JsResult<u64> {
    if !is_object(o) {
        return Err(error::throw_error(
            cx,
            ErrorKind::TypeError,
            "Array.prototype.join called on a non-object",
        ));
    }
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

/// Array.prototype.toString (§23.1.3.36): `join`, or Object.prototype.toString.
pub(crate) fn array_to_string<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    a: Args<'s>,
    nt: Local<'s>,
) -> JsResult<Local<'s>> {
    if is_array(this.bits()) {
        return join(cx.inner(), this.bits(), UNDEFINED).map(local);
    }
    object_to_string(cx, this, a, nt)
}

/// Array.prototype.push (§23.1.3.23, v0: arrays only).
pub(crate) fn array_push<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    let Some(a) = this.as_array() else {
        return Err(cx.throw_error(
            ErrorKind::TypeError,
            "not implemented in ep9 v0: Array.prototype.push on a non-array",
        ));
    };
    let a = a.bits();
    if crate::shape::proto_index_keys(cx.inner().rt()) {
        // A prototype may have an index key: each store is a `[[Set]]` walk.
        return push_with_set(cx.inner(), a, args).map(local);
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

/// Array.prototype.push steps 2-7 on the array `a` with `[[Set]]` for every store, so an
/// inherited setter, getter or read-only index property decides (`shape::proto_index_keys`).
/// Out of line, so the fast loop of [`array_push`] stays small.
#[cold]
#[inline(never)]
fn push_with_set(cx: &mut Cx, a: u64, args: Args<'_>) -> JsResult<u64> {
    // 2. Let len be ? LengthOfArrayLike(O).
    let mut len = f64::from(object::array_len(a));
    // 5. For each element E of items: Set(O, ! ToString(𝔽(len)), E, true); len = len + 1.
    for v in args.iter() {
        // GC point: `a` lives in a Rust local, `v` in the argument list.
        let key = conv::number_string(cx, len);
        property::set(cx, a, key, v.bits(), true)?;
        len += 1.0;
    }
    // 6. Perform ? Set(O, "length", 𝔽(len), true).
    let key = cx.rt().atom(Atom::Length);
    property::set(cx, a, key, num(len), true)?;
    // 7. Return 𝔽(len).
    Ok(num(len))
}
