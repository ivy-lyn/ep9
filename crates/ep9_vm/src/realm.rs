//! The v0 realm: intrinsics, the global object and a few builtins (M0 week 3). Week 4 brings
//! the builtin ABI (H20) and template builtins; these are only what the VM itself needs.
//!
//! | Global | Value |
//! |---|---|
//! | `globalThis`, `undefined`, `NaN`, `Infinity` | as in §19.1 |
//! | `Object`, `Array` | constructors (§20.1.1, §23.1.1; v0: no wrapper objects) |
//! | `Error`, `TypeError`, `RangeError`, `ReferenceError`, `SyntaxError`, `EvalError`, `URIError` | constructors (§20.5) |
//!
//! Prototype methods: `Object.prototype.toString`/`valueOf`, `Error.prototype.toString`,
//! `Array.prototype.join`/`toString`/`push`.

use ep9_value::{JsResult, Value};

use crate::conv::{self, num};
use crate::error;
use crate::frame::{NULL, UNDEFINED};
use crate::host::{Args, HostCx, HostFn};
use crate::interp::Cx;
use crate::object::{self, attr, is_array, is_function, is_object, own_define};
use crate::property;
use crate::runtime::{Atom, ErrorKind, Root, Runtime, val};
use crate::unit::FunctionInfo;

/// A host function object for `f`. Its FunctionInfo lives in `rt.infos` (boxed, stable).
pub(crate) fn new_host_function(
    rt: &Runtime,
    f: HostFn,
    name: &str,
    length: u32,
    constructor: bool,
) -> u64 {
    let info = Box::new(FunctionInfo::host(f, name, length, constructor));
    let p: *const FunctionInfo = &*info;
    rt.infos.borrow_mut().push(info);
    object::new_function(rt, p, UNDEFINED)
}

/// Defines `holder.name = <host function>` (not enumerable).
pub(crate) fn define_host_method(
    rt: &Runtime,
    holder: u64,
    name: &str,
    f: HostFn,
    length: u32,
) -> u64 {
    let func = new_host_function(rt, f, name, length, false);
    let key = object::new_string(rt, name);
    own_define(rt, holder, key, func, attr::HIDDEN);
    func
}

/// Builds the realm: atoms, intrinsics, the global object. Every new cell goes into a root
/// before the next allocation (GC-stress safe).
pub(crate) fn init(rt: &Runtime) {
    for &a in Atom::ALL {
        let s = object::new_string(rt, a.text());
        rt.set_atom(a, s);
    }
    let object_proto = object::new_object(rt, NULL);
    rt.set_root(Root::ObjectPrototype, object_proto);
    for (r, p) in [
        (Root::FunctionPrototype, object_proto),
        (Root::ArrayPrototype, object_proto),
        (Root::StringPrototype, object_proto),
        (Root::NumberPrototype, object_proto),
        (Root::BooleanPrototype, object_proto),
        (Root::ErrorPrototype, object_proto),
    ] {
        // v0: plain objects (Function.prototype is not callable, Array.prototype is not an
        // array exotic object).
        let o = object::new_object(rt, p);
        rt.set_root(r, o);
    }
    let lexical = object::new_object(rt, NULL);
    rt.set_root(Root::Lexical, lexical);
    let global = object::new_object(rt, object_proto);
    rt.set_root(Root::GlobalObject, global);

    own_define(rt, global, rt.atom(Atom::GlobalThis), global, attr::HIDDEN);
    own_define(rt, global, rt.atom(Atom::Undefined), UNDEFINED, 0);
    own_define(rt, global, rt.atom(Atom::NaN), num(f64::NAN), 0);
    own_define(rt, global, rt.atom(Atom::Infinity), num(f64::INFINITY), 0);

    define_host_method(rt, object_proto, "toString", object_to_string, 0);
    define_host_method(rt, object_proto, "valueOf", object_value_of, 0);
    let array_proto = rt.root(Root::ArrayPrototype);
    define_host_method(rt, array_proto, "join", array_join, 1);
    define_host_method(rt, array_proto, "toString", array_to_string, 0);
    define_host_method(rt, array_proto, "push", array_push, 1);

    // v0: one prototype for array and string iterators; `next` only (no Symbol.iterator).
    let iterator_proto = object::new_object(rt, object_proto);
    rt.set_root(Root::ArrayIteratorPrototype, iterator_proto);
    define_host_method(rt, iterator_proto, "next", iterator_next, 0);

    define_constructor(rt, global, "Object", object_constructor, object_proto);
    define_constructor(rt, global, "Array", array_constructor, array_proto);

    let error_proto = rt.root(Root::ErrorPrototype);
    define_host_method(rt, error_proto, "toString", error_to_string, 0);
    for kind in ErrorKind::ALL {
        let proto = if kind == ErrorKind::Error {
            error_proto
        } else {
            let p = object::new_object(rt, error_proto);
            rt.set_root(kind.prototype_root(), p);
            p
        };
        let name = object::new_string(rt, kind.name());
        own_define(rt, proto, rt.atom(Atom::Name), name, attr::HIDDEN);
        own_define(
            rt,
            proto,
            rt.atom(Atom::Message),
            rt.atom(Atom::Empty),
            attr::HIDDEN,
        );
        let ctor = new_host_function(rt, error_constructor(kind), kind.name(), 1, true);
        own_define(rt, ctor, rt.atom(Atom::Prototype), proto, 0);
        own_define(rt, proto, rt.atom(Atom::Constructor), ctor, attr::HIDDEN);
        let gname = object::new_string(rt, kind.name());
        own_define(rt, global, gname, ctor, attr::HIDDEN);
    }
}

/// Defines the global constructor `name` with `prototype` (and `prototype.constructor`).
fn define_constructor(rt: &Runtime, global: u64, name: &str, f: HostFn, proto: u64) {
    let ctor = new_host_function(rt, f, name, 1, true);
    own_define(rt, ctor, rt.atom(Atom::Prototype), proto, 0);
    own_define(rt, proto, rt.atom(Atom::Constructor), ctor, attr::HIDDEN);
    let key = object::new_string(rt, name);
    own_define(rt, global, key, ctor, attr::HIDDEN);
}

/// Object(value) (§20.1.1.1). v0: no wrapper objects, so a primitive gives a new object.
fn object_constructor(
    cx: &mut HostCx<'_>,
    _this: Value,
    args: Args<'_>,
    _nt: Value,
) -> JsResult<Value> {
    let v = args.get(0);
    if is_object(v.to_bits()) {
        return Ok(v);
    }
    Ok(val(object::new_plain_object(cx.cx.rt())))
}

/// Array(...items) and Array(length) (§23.1.1.1).
fn array_constructor(
    cx: &mut HostCx<'_>,
    _this: Value,
    args: Args<'_>,
    _nt: Value,
) -> JsResult<Value> {
    let rt = cx.cx.rt();
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
        return Ok(val(a));
    }
    let a = object::new_array(rt, args.len());
    for (i, v) in args.iter().enumerate() {
        object::array_set(rt, a, i as u32, v.to_bits());
    }
    Ok(val(a))
}

fn error_constructor(kind: ErrorKind) -> HostFn {
    macro_rules! ctor {
        ($k:ident) => {{
            fn f(
                cx: &mut HostCx<'_>,
                _this: Value,
                args: Args<'_>,
                new_target: Value,
            ) -> JsResult<Value> {
                construct_error(cx.cx, ErrorKind::$k, args.get(0), new_target)
            }
            f as HostFn
        }};
    }
    match kind {
        ErrorKind::Error => ctor!(Error),
        ErrorKind::TypeError => ctor!(TypeError),
        ErrorKind::RangeError => ctor!(RangeError),
        ErrorKind::ReferenceError => ctor!(ReferenceError),
        ErrorKind::SyntaxError => ctor!(SyntaxError),
        ErrorKind::EvalError => ctor!(EvalError),
        ErrorKind::UriError => ctor!(UriError),
    }
}

/// NativeError(message) (§20.5.6.1.1). v0: no `options.cause`.
fn construct_error(
    cx: &mut Cx,
    kind: ErrorKind,
    message: Value,
    new_target: Value,
) -> JsResult<Value> {
    let rt = cx.rt();
    let mut proto = rt.root(kind.prototype_root());
    let nt = new_target.to_bits();
    if is_function(nt) {
        let p = property::get(cx, nt, rt.atom(Atom::Prototype))?;
        if is_object(p) {
            proto = p;
        }
    }
    let msg = if message.is_undefined() {
        None
    } else {
        Some(conv::to_string(cx, message.to_bits())?)
    };
    Ok(val(error::new_error_value(cx, proto, msg)))
}

/// %ArrayIteratorPrototype%.next (§23.1.5.2.1, kind "value") and
/// %StringIteratorPrototype%.next (§22.1.5.2.1) for the v0 iterator cells.
fn iterator_next(cx: &mut HostCx<'_>, this: Value, _: Args<'_>, _: Value) -> JsResult<Value> {
    let cx = &mut *cx.cx;
    let it = this.to_bits();
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
        let units: Vec<u16> = {
            let s = object::str_view(target);
            let n = s.len();
            let at = i as usize;
            if at < n {
                let lead = s.at(at);
                let pair = (0xD800..0xDC00).contains(&lead)
                    && at + 1 < n
                    && (0xDC00..0xE000).contains(&s.at(at + 1));
                if pair {
                    vec![lead, s.at(at + 1)]
                } else {
                    vec![lead]
                }
            } else {
                Vec::new()
            }
        };
        if units.is_empty() {
            object::set_iterator_state(rt, it, UNDEFINED, i);
        } else {
            value = object::new_string_utf16(rt, &units);
            object::set_iterator_state(rt, it, target, i + units.len() as u32);
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
    Ok(val(result))
}

/// Error.prototype.toString (§20.5.3.4).
fn error_to_string(cx: &mut HostCx<'_>, this: Value, _: Args<'_>, _: Value) -> JsResult<Value> {
    let cx = &mut *cx.cx;
    let o = this.to_bits();
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
    Ok(val(object::new_string(rt, &s)))
}

/// Object.prototype.toString (§20.1.3.6, v0: no `@@toStringTag`).
fn object_to_string(cx: &mut HostCx<'_>, this: Value, _: Args<'_>, _: Value) -> JsResult<Value> {
    let o = this.to_bits();
    let tag = match o {
        UNDEFINED => "Undefined",
        NULL => "Null",
        _ if is_array(o) => "Array",
        _ if is_function(o) => "Function",
        _ if object::is_error(o) => "Error",
        _ if object::is_string(o) => "String",
        _ if val(o).is_number() => "Number",
        _ if val(o).is_bool() => "Boolean",
        _ => "Object",
    };
    Ok(cx.new_string(&format!("[object {tag}]")))
}

/// Object.prototype.valueOf (§20.1.3.7, v0: primitives are not wrapped).
fn object_value_of(cx: &mut HostCx<'_>, this: Value, _: Args<'_>, _: Value) -> JsResult<Value> {
    if this.is_undefined_or_null() {
        return Err(cx.throw_error(
            ErrorKind::TypeError,
            "Cannot convert undefined or null to object",
        ));
    }
    Ok(this)
}

/// Array.prototype.join (§23.1.3.18).
fn array_join(cx: &mut HostCx<'_>, this: Value, args: Args<'_>, _: Value) -> JsResult<Value> {
    join(cx.cx, this.to_bits(), args.get(0).to_bits())
}

fn join(cx: &mut Cx, o: u64, sep: u64) -> JsResult<Value> {
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
    Ok(val(acc))
}

fn concat_or_throw(cx: &mut Cx, a: u64, b: u64) -> JsResult<u64> {
    object::concat(cx.rt(), a, b)
        .ok_or_else(|| error::throw_error(cx, ErrorKind::RangeError, "Invalid string length"))
}

/// Array.prototype.toString (§23.1.3.36): `join`, or Object.prototype.toString.
fn array_to_string(cx: &mut HostCx<'_>, this: Value, a: Args<'_>, nt: Value) -> JsResult<Value> {
    if is_array(this.to_bits()) {
        return join(cx.cx, this.to_bits(), UNDEFINED);
    }
    object_to_string(cx, this, a, nt)
}

/// Array.prototype.push (§23.1.3.23, v0: arrays only).
fn array_push(cx: &mut HostCx<'_>, this: Value, args: Args<'_>, _: Value) -> JsResult<Value> {
    let a = this.to_bits();
    if !is_array(a) {
        return Err(cx.throw_error(
            ErrorKind::TypeError,
            "not implemented in ep9 v0: Array.prototype.push on a non-array",
        ));
    }
    for v in args.iter() {
        let n = object::array_len(a);
        if !object::array_set(cx.cx.rt(), a, n, v.to_bits()) {
            return Err(cx.throw_error(ErrorKind::RangeError, "Invalid array length"));
        }
    }
    Ok(val(num(f64::from(object::array_len(a)))))
}

/// The global declaration step of `DeclareGlobals` (§16.1.7 GlobalDeclarationInstantiation,
/// v0): `var`/function names become global properties (undefined if new); `let`/`const`/
/// `class` names go to the lexical record as holes (TDZ; `const` is not writable).
/// `InitializeGlobal` stores their values later. Conflicts throw a SyntaxError.
pub(crate) fn declare_globals(
    cx: &mut Cx,
    unit: &ep9_bytecode::CompiledUnit,
    strings: &dyn Fn(usize) -> u64,
) -> conv::Res<()> {
    use ep9_bytecode::{ScopeKind, VariableMode};
    let rt = cx.rt();
    let Some(scope) = unit
        .scopes
        .iter()
        .find(|s| matches!(s.kind, ScopeKind::Script | ScopeKind::Eval))
    else {
        return Ok(());
    };
    let global = rt.global();
    let lexical = rt.root(Root::Lexical);
    for s in &scope.slots {
        let name = strings(s.name.index());
        let lexical_decl = matches!(
            s.mode,
            VariableMode::Let | VariableMode::Const | VariableMode::Class
        );
        let clash = object::own_get(lexical, name).is_some()
            || lexical_decl
                && object::own_get(global, name).is_some_and(|(_, a)| a & attr::CONFIGURABLE == 0);
        if clash {
            let n = object::str_view(name).to_string_lossy();
            return Err(error::throw_error(
                cx,
                ErrorKind::SyntaxError,
                &format!("Identifier '{n}' has already been declared"),
            ));
        }
        if lexical_decl {
            let a = if s.mode == VariableMode::Const {
                0
            } else {
                attr::WRITABLE
            };
            own_define(rt, lexical, name, crate::frame::HOLE, a);
        } else if object::own_get(global, name).is_none() {
            own_define(
                rt,
                global,
                name,
                UNDEFINED,
                attr::WRITABLE | attr::ENUMERABLE,
            );
        }
    }
    Ok(())
}
