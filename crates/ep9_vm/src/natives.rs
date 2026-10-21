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
//! | %ArrayIteratorPrototype%.next, %StringIteratorPrototype%.next | §23.1.5.2.1, §22.1.5.2.1 |
//! | `Object.prototype.toString`, `valueOf` | §20.1.3.6, §20.1.3.7 |
//! | `Error.prototype.toString` | §20.5.3.4 |
//! | `Array.prototype.join`, `toString`, `push` | §23.1.3.18, §23.1.3.36, §23.1.3.23 |

use ep9_value::JsResult;

use crate::abi::{Args, Cx as HostCx, Local};
use crate::atoms::Atom;
use crate::builtins::{
    BuiltinDef, BuiltinModule, BuiltinTable, ConstValue, IntrinsicDef, IntrinsicId, IntrinsicValue,
    ObjectProps, PropAttrs, PropDef, PropKey,
};
use crate::conv::{self, num};
use crate::error;
use crate::frame::{NULL, UNDEFINED};
use crate::interp::Cx;
use crate::kinds::is_callable;
use crate::object::{self, attr, is_array, is_object, own_define};
use crate::property;
use crate::runtime::ErrorKind;
use crate::{string, wrapper};

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
];

const PROTOTYPE: PropKey = PropKey::Atom(Atom::Prototype);
const CONSTRUCTOR: PropKey = PropKey::Atom(Atom::Constructor);
const TO_STRING: PropKey = PropKey::Atom(Atom::ToStringKey);

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
        ],
    },
    ObjectProps {
        target: IntrinsicId::Array,
        props: &prototype_of(IntrinsicId::ArrayPrototype),
    },
    ObjectProps {
        target: IntrinsicId::ArrayIteratorPrototype,
        props: &[PropDef::method(
            PropKey::Atom(Atom::Next),
            vm::ITERATOR_NEXT,
        )],
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

/// %ArrayIteratorPrototype%.next (§23.1.5.2.1, kind "value") and
/// %StringIteratorPrototype%.next (§22.1.5.2.1) for the v0 iterator cells.
pub(crate) fn iterator_next<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    _: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    let cx = cx.inner();
    let it = this.bits();
    if !object::is_iterator(it) {
        return Err(error::throw_error(
            cx,
            ErrorKind::TypeError,
            "next method called on an incompatible receiver",
        ));
    }
    let rt = cx.rt();
    let (target, i) = object::iterator_state(it);
    let mut value = UNDEFINED;
    let mut done = true;
    if object::is_string(target) {
        // The code point at `i`: one code unit, or a surrogate pair (§22.1.5.2.1).
        let (n, pair) = {
            let s = object::str_view(target);
            let n = s.len();
            let at = i as usize;
            let pair = at + 1 < n
                && (0xD800..0xDC00).contains(&s.at(at))
                && (0xDC00..0xE000).contains(&s.at(at + 1));
            (n, pair.then(|| [s.at(at), s.at(at + 1)]))
        };
        if i as usize >= n {
            object::set_iterator_state(rt, it, UNDEFINED, i);
        } else {
            let len = match pair {
                Some(units) => {
                    value = object::new_string_utf16(rt, &units);
                    2
                }
                None => {
                    value = string::char_at(rt, target, i as usize);
                    1
                }
            };
            object::set_iterator_state(rt, it, target, i + len);
            done = false;
        }
    } else if is_array(target) {
        if i < object::array_len(target) {
            value = property::get_keyed(cx, target, num(f64::from(i)))?;
            object::set_iterator_state(rt, it, target, i + 1);
            done = false;
        } else {
            object::set_iterator_state(rt, it, UNDEFINED, i);
        }
    }
    // CreateIterResultObject(value, done) (§7.4.14).
    let result = object::new_plain_object(rt);
    own_define(rt, result, rt.atom(Atom::ValueKey), value, attr::DEFAULT);
    let done = if done {
        crate::frame::TRUE
    } else {
        crate::frame::FALSE
    };
    own_define(rt, result, rt.atom(Atom::Done), done, attr::DEFAULT);
    Ok(local(result))
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

/// Object.prototype.toString (§20.1.3.6, v0: no `@@toStringTag`).
pub(crate) fn object_to_string<'s>(
    cx: &mut HostCx<'s>,
    this: Local<'s>,
    _: Args<'s>,
    _: Local<'s>,
) -> JsResult<Local<'s>> {
    let o = this.bits();
    // 3. O = ! ToObject(this value): a primitive and its wrapper have the same tag, from
    //    [[BooleanData]], [[NumberData]] or [[StringData]] (steps 9-11).
    let data = local(wrapper::wrapper_value(o).unwrap_or(o));
    let tag = match o {
        UNDEFINED => "Undefined",
        NULL => "Null",
        _ if is_array(o) => "Array",
        _ if is_callable(o) => "Function",
        _ if object::is_error(o) => "Error",
        _ if data.is_bool() => "Boolean",
        _ if data.is_number() => "Number",
        _ if data.is_string() => "String",
        _ => "Object",
    };
    Ok(cx.new_string(&format!("[object {tag}]"))?.value())
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
