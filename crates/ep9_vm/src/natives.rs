//! The host functions of the v0 realm (realm.rs installs them), on the builtin ABI v1 (H20).
//! They live in ep9_vm because they read VM internals (iterator cells, error sites); the
//! builtin tables (P2) take them over.
//!
//! | Function | Spec |
//! |---|---|
//! | `Object`, `Array` | §20.1.1.1, §23.1.1.1 |
//! | `Error` and the native errors | §20.5.1.1, §20.5.6.1.1 |
//! | %ArrayIteratorPrototype%.next, %StringIteratorPrototype%.next | §23.1.5.2.1, §22.1.5.2.1 |
//! | `Object.prototype.toString`, `valueOf` | §20.1.3.6, §20.1.3.7 |
//! | `Error.prototype.toString` | §20.5.3.4 |
//! | `Array.prototype.join`, `toString`, `push` | §23.1.3.18, §23.1.3.36, §23.1.3.23 |

use ep9_value::JsResult;

use crate::abi::{Args, Cx as HostCx, HostFn, Local};
use crate::conv::{self, num};
use crate::error;
use crate::frame::{NULL, UNDEFINED};
use crate::interp::Cx;
use crate::kinds::is_callable;
use crate::object::{self, attr, is_array, is_object, own_define};
use crate::property;
use crate::runtime::{Atom, ErrorKind};
use crate::string;

/// A `Local` of VM bits.
fn local<'s>(b: u64) -> Local<'s> {
    Local::from_bits(b)
}

/// Object(value) (§20.1.1.1). v0: no wrapper objects, so a primitive gives a new object.
pub(crate) fn object_constructor<'s>(
    cx: &mut HostCx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let v = args.get(0);
    if v.is_object() {
        return Ok(v);
    }
    Ok(cx.new_object().value())
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

pub(crate) fn error_constructor(kind: ErrorKind) -> HostFn {
    macro_rules! ctor {
        ($k:ident) => {{
            fn f<'s>(
                cx: &mut HostCx<'s>,
                _this: Local<'s>,
                args: Args<'s>,
                new_target: Local<'s>,
            ) -> JsResult<Local<'s>> {
                let (m, nt) = (args.get(0).bits(), new_target.bits());
                construct_error(cx.inner(), ErrorKind::$k, m, nt).map(local)
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
    let tag = match o {
        UNDEFINED => "Undefined",
        NULL => "Null",
        _ if is_array(o) => "Array",
        _ if is_callable(o) => "Function",
        _ if object::is_error(o) => "Error",
        _ if object::is_string(o) => "String",
        _ if this.is_number() => "Number",
        _ if this.is_bool() => "Boolean",
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
