//! The host functions of the v0 realm (realm.rs installs them). Week 4 moves them to the
//! builtin ABI (H20) and the builtin tables.
//!
//! | Function | Spec |
//! |---|---|
//! | `Object`, `Array` | §20.1.1.1, §23.1.1.1 |
//! | `Error` and the native errors | §20.5.1.1, §20.5.6.1.1 |
//! | %ArrayIteratorPrototype%.next, %StringIteratorPrototype%.next | §23.1.5.2.1, §22.1.5.2.1 |
//! | `Object.prototype.toString`, `valueOf` | §20.1.3.6, §20.1.3.7 |
//! | `Error.prototype.toString` | §20.5.3.4 |
//! | `Array.prototype.join`, `toString`, `push` | §23.1.3.18, §23.1.3.36, §23.1.3.23 |

use ep9_value::{JsResult, Value};

use crate::conv::{self, num};
use crate::error;
use crate::frame::{NULL, UNDEFINED};
use crate::host::{Args, HostCx, HostFn};
use crate::interp::Cx;
use crate::kinds::is_callable;
use crate::object::{self, attr, is_array, is_object, own_define};
use crate::property;
use crate::runtime::{Atom, ErrorKind, val};
use crate::string;

/// Object(value) (§20.1.1.1). v0: no wrapper objects, so a primitive gives a new object.
pub(crate) fn object_constructor(
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
pub(crate) fn array_constructor(
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

pub(crate) fn error_constructor(kind: ErrorKind) -> HostFn {
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
    if is_callable(nt) {
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
pub(crate) fn iterator_next(
    cx: &mut HostCx<'_>,
    this: Value,
    _: Args<'_>,
    _: Value,
) -> JsResult<Value> {
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
    Ok(val(result))
}

/// Error.prototype.toString (§20.5.3.4).
pub(crate) fn error_to_string(
    cx: &mut HostCx<'_>,
    this: Value,
    _: Args<'_>,
    _: Value,
) -> JsResult<Value> {
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
pub(crate) fn object_to_string(
    cx: &mut HostCx<'_>,
    this: Value,
    _: Args<'_>,
    _: Value,
) -> JsResult<Value> {
    let o = this.to_bits();
    let tag = match o {
        UNDEFINED => "Undefined",
        NULL => "Null",
        _ if is_array(o) => "Array",
        _ if is_callable(o) => "Function",
        _ if object::is_error(o) => "Error",
        _ if object::is_string(o) => "String",
        _ if val(o).is_number() => "Number",
        _ if val(o).is_bool() => "Boolean",
        _ => "Object",
    };
    Ok(cx.new_string(&format!("[object {tag}]")))
}

/// Object.prototype.valueOf (§20.1.3.7, v0: primitives are not wrapped).
pub(crate) fn object_value_of(
    cx: &mut HostCx<'_>,
    this: Value,
    _: Args<'_>,
    _: Value,
) -> JsResult<Value> {
    if this.is_undefined_or_null() {
        return Err(cx.throw_error(
            ErrorKind::TypeError,
            "Cannot convert undefined or null to object",
        ));
    }
    Ok(this)
}

/// Array.prototype.join (§23.1.3.18).
pub(crate) fn array_join(
    cx: &mut HostCx<'_>,
    this: Value,
    args: Args<'_>,
    _: Value,
) -> JsResult<Value> {
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
pub(crate) fn array_to_string(
    cx: &mut HostCx<'_>,
    this: Value,
    a: Args<'_>,
    nt: Value,
) -> JsResult<Value> {
    if is_array(this.to_bits()) {
        return join(cx.cx, this.to_bits(), UNDEFINED);
    }
    object_to_string(cx, this, a, nt)
}

/// Array.prototype.push (§23.1.3.23, v0: arrays only).
pub(crate) fn array_push(
    cx: &mut HostCx<'_>,
    this: Value,
    args: Args<'_>,
    _: Value,
) -> JsResult<Value> {
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
