//! The internal methods of ordinary objects (§10.1) on shapes (H17 v0): `[[Get]]`, `[[Set]]`,
//! `[[DefineOwnProperty]]`, `[[Delete]]`, `[[HasProperty]]`, `[[GetOwnProperty]]`,
//! `[[OwnPropertyKeys]]`, `[[GetPrototypeOf]]`, `[[SetPrototypeOf]]`, `[[IsExtensible]]` and
//! `[[PreventExtensions]]`, plus the exotic parts of arrays and String wrappers
//! (`exotic.rs`). Data and accessor properties (`accessor.rs`: an accessor stores an
//! immutable ACCESSOR_PAIR); v0: dense array elements only, and every element is a data
//! property {w, e, c}.
//!
//! ```text
//!  get / set / has / delete
//!    probe(key) once ──> per object on the chain: exotic::get / own_attrs ──> props (shape or table)
//!    an accessor on the chain: get calls its getter, set its setter, with the receiver as `this`
//!  define (CreateDataPropertyOrThrow)
//!    fast path: a new key on an extensible object, or an existing configurable data property
//!    otherwise: define_own_property (ValidateAndApplyPropertyDescriptor), TypeError on false
//! ```
//!
//! | Receiver | Own properties seen before the storage |
//! |---|---|
//! | array | `length` {w}, array-index keys (elements, {w, e, c}) |
//! | constructor function | `prototype` (created on first use, `exotic.rs`) |
//! | primitive wrapper | `wrapper::string_own_property` (String wrappers: `length`, indices) |
//! | arguments object, module namespace | routed hooks (`exotic.rs`): `arguments.rs`, `module.rs` |
//! | string primitive | `length`, array-index keys; then `String.prototype` |
//! | number / boolean primitive | `Number.prototype` / `Boolean.prototype` |
//! | symbol primitive | `Symbol.prototype` (`symbol::prototype`) |
//!
//! The builtin ABI (H20 ObjectOps, abi/mod.rs) calls [`get_own_property`], [`own_property_keys`],
//! [`define_own_property`] (ValidateAndApplyPropertyDescriptor) and [`get_index`] and its kin.
//!
//! The kinds with exotic internal methods (ARGUMENTS, MODULE_NAMESPACE; OBJECT_LAYOUT 3) are
//! routed at the start of `[[Set]]`, `[[DefineOwnProperty]]`, `[[Delete]]` and
//! `[[OwnPropertyKeys]]` (`exotic::set` and its kin: one kind test), and at each routed
//! prototype of the `[[Set]]` walk (§10.1.9.2 step 1.b). The integer-index paths
//! ([`get_index`] and its kin) send a routed object to the key path, and `for-in`
//! ([`for_in_keys`]) asks its `[[OwnPropertyKeys]]` and `[[GetOwnProperty]]`. A hook that answers
//! `None` takes the ordinary method; a hook that needs the ordinary step calls
//! [`ordinary_set_with_receiver`], [`ordinary_define_own_property`], [`ordinary_delete`] or
//! [`ordinary_own_property_keys`].

use crate::conv::{Res, describe, num, to_property_key, to_uint32_of};
use crate::descriptor::Desc;
use crate::error::{not_implemented, throw_error};
use crate::exotic::{self, Exotic};
use crate::frame::{NULL, UNDEFINED};
use crate::interp::Cx;
use crate::key::{
    self, Probe, array_index_of, intern_key, is_property_key, is_symbol_key, key_text,
};
use crate::kinds::{self as kind, is_bound, is_callable, is_symbol};
use crate::object::{self, attr, is_array, is_function, is_object, is_string};
use crate::props;
use crate::runtime::{Atom, ErrorKind, Root, Runtime, val};
use crate::{accessor, bound, shape, string, symbol, wrapper};

fn is_atom(cx: &Cx, key: u64, a: Atom) -> bool {
    object::key_eq(key, cx.rt().atom(a))
}

/// OrdinaryGet over the prototype chain, starting at object `o` (receiver `o`).
pub(crate) fn get_from(cx: &mut Cx, o: u64, key: u64) -> Res<u64> {
    get_with_receiver(cx, o, key, o)
}

/// `o.[[Get]](key, receiver)` (§10.1.8.1 OrdinaryGet) up the prototype chain from the
/// object `o`. The probe is classified once. An accessor found on the chain calls its getter
/// with `receiver` as `this` (any value: a primitive base passes itself). GC point (a lazy
/// `prototype`, a getter).
pub(crate) fn get_with_receiver(cx: &mut Cx, mut o: u64, key: u64, receiver: u64) -> Res<u64> {
    debug_assert!(is_object(o) && is_property_key(key));
    let how = key::probe(key);
    loop {
        if kind::cell_kind(o) != kind::OBJECT.get()
            && let Some(r) = exotic::get(cx, o, key, how)
        {
            return r;
        }
        match props::own_value_or_proto(o, key, how) {
            Ok((v, a)) if a & attr::ACCESSOR == 0 => return Ok(v),
            Ok((pair, _)) => return accessor::call_getter(cx, pair, receiver),
            Err(NULL) => return Ok(UNDEFINED),
            Err(p) => o = p,
        }
    }
}

/// The value of the first property `key` on the chain from the object `o` if it is a data
/// property; `None` for an accessor (its getter does not run) or no property. For engine
/// text that must run no JS: an error's "Name: message" header (`error::error_header`, made
/// inside `throw_error`, also on stack overflow) and the reported constructor name. GC point
/// (a lazy `prototype`), but runs no JS and cannot throw. A routed object's hooks
/// (ARGUMENTS, MODULE_NAMESPACE) are not asked: only its ordinary storage is read, then the
/// walk goes on to its prototype. A hook may throw (a namespace binding in its TDZ), and the
/// ReferenceError it makes would read its own header here again (`error::error_header` ->
/// `get_data_from`) and recurse without bound.
pub(crate) fn get_data_from(cx: &mut Cx, mut o: u64, key: u64) -> Option<u64> {
    debug_assert!(is_object(o) && is_property_key(key));
    let how = key::probe(key);
    loop {
        let k = kind::cell_kind(o);
        if k != kind::OBJECT.get() && !exotic::is_routed(k) {
            // Only non-routed kinds get here, and their `exotic::get` never throws; the
            // `Some(Err)` arm is defensive.
            match exotic::get(cx, o, key, how) {
                Some(Ok(v)) => return Some(v),
                Some(Err(_)) => {
                    cx.rt().take_pending();
                    return None;
                }
                None => {}
            }
        }
        match props::own_value_or_proto(o, key, how) {
            Ok((v, a)) => return (a & attr::ACCESSOR == 0).then_some(v),
            Err(NULL) => return None,
            Err(p) => o = p,
        }
    }
}

/// `base[key]` with a property key (GetValue on a property reference).
#[inline]
pub(crate) fn get(cx: &mut Cx, base: u64, key: u64) -> Res<u64> {
    debug_assert!(is_property_key(key));
    if is_object(base) {
        return get_from(cx, base, key);
    }
    get_primitive(cx, base, key)
}

/// [`get`] on a primitive `base`: a string's own `length` and indices, then the chain of its
/// prototype with the primitive as the receiver (undefined / null: TypeError). Out of line,
/// so `get` stays small.
#[inline(never)]
fn get_primitive(cx: &mut Cx, base: u64, key: u64) -> Res<u64> {
    let rt = cx.rt();
    if is_string(base) {
        if is_atom(cx, key, Atom::Length) {
            return Ok(num(object::str_len(base) as f64));
        }
        if let Some(i) = array_index_of(key)
            && (i as usize) < object::str_len(base)
        {
            return Ok(string::char_at(rt, base, i as usize));
        }
    }
    let Some(proto) = primitive_proto(rt, base) else {
        let k = key_text(rt, key);
        let msg = format!(
            "Cannot read properties of {} (reading '{k}')",
            describe(base)
        );
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    };
    get_with_receiver(cx, proto, key, base)
}

/// The `[[Prototype]]` of `ToObject(b)` for a primitive `b` (§7.1.18): where a property
/// access on a string, number, boolean or symbol goes after the primitive's own properties.
/// `None` for undefined and null. Not a GC point.
fn primitive_proto(rt: &Runtime, b: u64) -> Option<u64> {
    if is_string(b) {
        Some(rt.root(Root::StringPrototype))
    } else if val(b).is_number() {
        Some(rt.root(Root::NumberPrototype))
    } else if b == object_bool(true) || b == object_bool(false) {
        Some(rt.root(Root::BooleanPrototype))
    } else if is_symbol(b) {
        // Made by `symbol::install` in realm init, before any script runs.
        Some(symbol::prototype(rt))
    } else {
        None
    }
}

#[inline]
fn object_bool(b: bool) -> u64 {
    crate::conv::boolean(b)
}

/// A string, number, boolean or symbol in V8's message form: `string 'ab'`, `number '1.5'`,
/// `boolean 'true'`, `symbol 'Symbol(x)'`.
fn primitive_text(b: u64) -> String {
    let (ty, text) = if is_string(b) {
        ("string", string::to_rust_string(b))
    } else if is_symbol(b) {
        ("symbol", describe(b))
    } else if val(b).is_number() {
        ("number", describe(b))
    } else {
        ("boolean", describe(b))
    };
    format!("{ty} '{text}'")
}

/// `base[k]` for any key value (keyed load).
pub(crate) fn get_keyed(cx: &mut Cx, base: u64, k: u64) -> Res<u64> {
    if let Some(i) = val(k).as_int32()
        && i >= 0
    {
        if is_array(base) {
            if let Some(v) = object::array_get(base, i as u32) {
                return Ok(v);
            }
        } else if is_string(base) && (i as usize) < object::str_len(base) {
            return Ok(string::char_at(cx.rt(), base, i as usize));
        }
    }
    if base == UNDEFINED || base == NULL {
        let msg = format!(
            "Cannot read properties of {} (reading '{}')",
            describe(base),
            describe(k).trim_matches('"')
        );
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    let key = to_property_key(cx, k)?;
    get(cx, base, key)
}

/// PutValue on a property reference (§6.2.5.6): `base[key] = v` with receiver `base`. A
/// failed assignment throws a TypeError in strict code. GC point (a setter runs JS).
pub(crate) fn set(cx: &mut Cx, base: u64, key: u64, v: u64, strict: bool) -> Res<()> {
    debug_assert!(is_property_key(key));
    if is_object(base) {
        // OrdinarySet with the walk inlined: `set` is hot until the ICs.
        if set_walk(cx, base, key, v, base)? {
            return Ok(());
        }
        return fail(cx, base, base, key, strict);
    }
    set_primitive(cx, base, key, v, strict)
}

/// [`set`] on a primitive `base` (or undefined / null: TypeError). Kept out of line so `set`
/// inlines its one call of [`set_with_receiver`]. GC point.
#[cold]
#[inline(never)]
fn set_primitive(cx: &mut Cx, base: u64, key: u64, v: u64, strict: bool) -> Res<()> {
    let Some(proto) = primitive_proto(cx.rt(), base) else {
        let msg = format!(
            "Cannot set properties of {} (setting '{}')",
            describe(base),
            key_text(cx.rt(), key)
        );
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    };
    // Step 5.a: ToObject(base).[[Set]](key, v, base). The wrapper's own properties come
    // first: a String's `length` and indices are not writable (§10.4.3); the other wrappers
    // have none. Then the prototype chain decides; a setter gets the primitive as `this`, and
    // a data property fails because the receiver is not an object (§10.1.9.2 step 2.c).
    let own = is_string(base)
        && (is_atom(cx, key, Atom::Length)
            || array_index_of(key).is_some_and(|i| (i as usize) < object::str_len(base)));
    if !own && set_with_receiver(cx, proto, key, v, base)? {
        return Ok(());
    }
    fail(cx, base, proto, key, strict)
}

/// `o.[[Set]](key, v, o)` (§10.1.9 OrdinarySet, receiver = `o`) on an object. `Ok(false)`
/// when the assignment fails (a non-writable property on the chain, or a new property on a
/// non-extensible object).
pub(crate) fn ordinary_set(cx: &mut Cx, o: u64, key: u64, v: u64) -> Res<bool> {
    set_with_receiver(cx, o, key, v, o)
}

/// `o.[[Set]](key, v, receiver)` (§10.1.9.1 OrdinarySet and §10.1.9.2
/// OrdinarySetWithOwnDescriptor) on the object `o`. `Ok(false)` when the assignment fails: a
/// non-writable data property or an accessor without a setter on the chain, a receiver that
/// is not an object, or a new property on a non-extensible receiver. An accessor calls its
/// setter with `receiver` as `this`. GC point.
pub(crate) fn set_with_receiver(cx: &mut Cx, o: u64, key: u64, v: u64, receiver: u64) -> Res<bool> {
    set_walk(cx, o, key, v, receiver)
}

/// OrdinarySet (§10.1.9.1) on the object `o`, past the `[[Set]]` hook of `o` (`exotic::set`):
/// the ordinary step of a routed kind's `[[Set]]`. GC point.
#[allow(dead_code)] // first caller: M3-P2 VM-ARGS (arguments.rs)
pub(crate) fn ordinary_set_with_receiver(
    cx: &mut Cx,
    o: u64,
    key: u64,
    v: u64,
    receiver: u64,
) -> Res<bool> {
    ordinary_set_walk(cx, o, key, v, receiver)
}

/// The body of [`set_with_receiver`], inlined into [`set`]: the `[[Set]]` hook of a routed
/// kind (`exotic::set`), then OrdinarySet.
#[inline(always)]
fn set_walk(cx: &mut Cx, o: u64, key: u64, v: u64, receiver: u64) -> Res<bool> {
    debug_assert!(is_object(o) && is_property_key(key));
    if let Some(r) = exotic::set(cx, o, key, v, receiver) {
        return r;
    }
    ordinary_set_walk(cx, o, key, v, receiver)
}

/// OrdinarySet (§10.1.9.1-2) on the object `o`.
#[inline(always)]
fn ordinary_set_walk(cx: &mut Cx, o: u64, key: u64, v: u64, receiver: u64) -> Res<bool> {
    debug_assert!(is_object(o) && is_property_key(key));
    let rt = cx.rt();
    let how = key::probe(key);
    if receiver == o {
        if is_array(o) {
            if exotic::is_key(rt, key, how, Atom::Length) {
                let n = to_uint32_of(cx, v)?;
                let d = crate::conv::to_number(cx, v)?;
                if f64::from(n) != d || !object::array_set_length(rt, o, n) {
                    return Err(throw_error(
                        cx,
                        ErrorKind::RangeError,
                        "Invalid array length",
                    ));
                }
                return Ok(true);
            }
            if let Some(i) = array_index_of(key)
                && array_store(rt, o, i, v)
            {
                return Ok(true);
            }
        } else if is_function(o) && exotic::is_lazy_prototype(rt, o, key, how) {
            // Make `prototype`, so the store below replaces it.
            exotic::function_prototype(rt, o);
        }
    }
    // The first property on the chain decides (§10.1.9.2 steps 1-2 and 4-7).
    let mut cur = o;
    loop {
        let found = match exotic::own_attrs(rt, cur, key, how) {
            // Exotic own properties are data properties: the value is not needed.
            Some(a) => Some((UNDEFINED, a)),
            // The common case, in one lookup: an own data property of the receiver takes the
            // value in place if it is writable (array `length` and elements returned above;
            // a lazy `prototype` is made, so it is stored). A read-only one or an accessor
            // (no WRITABLE bit) is looked up again.
            None if cur == receiver => match props::own_store_if_writable(rt, cur, key, how, v) {
                Some(true) => return Ok(true),
                Some(false) => props::own_get_with(cur, key, how),
                None => None,
            },
            None => props::own_get_with(cur, key, how),
        };
        if let Some((pv, a)) = found {
            if a & attr::ACCESSOR != 0 {
                return accessor::call_setter(cx, pv, receiver, v);
            }
            if a & attr::WRITABLE == 0 {
                return Ok(false);
            }
            break;
        }
        let p = object::proto_of(cur);
        if p == NULL {
            break;
        }
        // Step 1.b: parent.[[Set]](P, V, Receiver). A routed prototype's hook decides.
        if exotic::is_routed(kind::cell_kind(p)) {
            return routed_proto_set(cx, p, key, v, receiver);
        }
        cur = p;
    }
    define_on_receiver(cx, receiver, key, v)
}

/// §10.1.9.2 step 1.b on a routed prototype `p` (ARGUMENTS, MODULE_NAMESPACE) of the
/// `[[Set]]` walk: `p.[[Set]](key, v, receiver)`. Its hook decides; `None` is OrdinarySet from
/// `p`. Out of line: it breaks the inlined cycle `set_walk` -> `ordinary_set_walk` ->
/// `set_walk`. GC point: `p`, `key`, `v` and `receiver` live in Rust locals.
#[cold]
#[inline(never)]
fn routed_proto_set(cx: &mut Cx, p: u64, key: u64, v: u64, receiver: u64) -> Res<bool> {
    set_with_receiver(cx, p, key, v, receiver)
}

/// §10.1.9.2 steps 2.c-e: the store lands on `receiver`: an existing writable data property
/// gets the value, a missing one is CreateDataProperty. An array receiver's `length` (which
/// `own_property` does not see) is an existing writable data property: step 2.d.iv goes to
/// ArraySetLength (a RangeError for an invalid length). GC point.
fn define_on_receiver(cx: &mut Cx, receiver: u64, key: u64, v: u64) -> Res<bool> {
    if !is_object(receiver) {
        return Ok(false);
    }
    if is_array(receiver) && is_atom(cx, key, Atom::Length) {
        let desc = Desc {
            value: Some(v),
            ..Desc::default()
        };
        return define_own_property(cx, receiver, key, desc);
    }
    match own_property(cx, receiver, key)? {
        Some((_, a, _)) if a & attr::WRITABLE == 0 => Ok(false),
        Some(_) => {
            let desc = Desc {
                value: Some(v),
                ..Desc::default()
            };
            define_own_property(cx, receiver, key, desc)
        }
        None => {
            if !is_extensible(receiver) {
                return Ok(false);
            }
            add_own_property(cx, receiver, key, v, attr::DEFAULT)
        }
    }
}

/// `a[i] = v` on the array `a` (receiver `a`) without the walk: an existing element is a
/// writable own data property (§10.1.9.2 step 2.d). A missing one is added (step 2.e) only
/// if `a` is extensible and no prototype has an index key (`shape::proto_index_keys`).
/// Otherwise the caller takes the walk (step 1.b), where an inherited setter, a getter
/// without a setter or a read-only property `i` decides. False if it stored nothing (also
/// past the v0 dense limit).
fn array_store(rt: &Runtime, a: u64, i: u32, v: u64) -> bool {
    if object::array_get(a, i).is_none() && (!is_extensible(a) || shape::proto_index_keys(rt)) {
        return false;
    }
    object::array_set(rt, a, i, v)
}

/// A failed `base[key] = v`: nothing in sloppy code, a TypeError in strict code. The message
/// names the first property on the chain from `start` (`base`, or the prototype of a
/// primitive `base`), as V8 does. Not a GC point before the throw.
#[cold]
#[inline(never)]
fn fail(cx: &mut Cx, base: u64, start: u64, key: u64, strict: bool) -> Res<()> {
    if !strict {
        return Ok(());
    }
    let rt = cx.rt();
    let k = key_text(rt, key);
    let what = if is_object(base) {
        "#<Object>".to_string()
    } else {
        primitive_text(base)
    };
    let own_string = is_string(base)
        && (is_atom(cx, key, Atom::Length)
            || array_index_of(key).is_some_and(|i| (i as usize) < object::str_len(base)));
    let found = if own_string {
        Some(0)
    } else {
        first_attrs_on_chain(rt, start, key)
    };
    let msg = match found {
        Some(a) if a & attr::ACCESSOR != 0 => {
            format!("Cannot set property {k} of {what} which has only a getter")
        }
        Some(a) if a & attr::WRITABLE == 0 && is_object(base) => {
            format!("Cannot assign to read only property '{k}' of object '{what}'")
        }
        Some(a) if a & attr::WRITABLE == 0 => {
            format!("Cannot assign to read only property '{k}' of {what}")
        }
        _ if is_object(base) => format!("Cannot add property {k}, object is not extensible"),
        _ => format!("Cannot create property '{k}' on {what}"),
    };
    Err(throw_error(cx, ErrorKind::TypeError, &msg))
}

/// The attributes of the first property `key` on the chain from the object `o`, if any.
/// Not a GC point (a lazy `prototype` is not made: it is {w}).
fn first_attrs_on_chain(rt: &Runtime, mut o: u64, key: u64) -> Option<i32> {
    let how = key::probe(key);
    loop {
        if let Some(a) = exotic::own_attrs(rt, o, key, how)
            .or_else(|| props::own_get_with(o, key, how).map(|(_, a)| a))
        {
            return Some(a);
        }
        o = object::proto_of(o);
        if o == NULL {
            return None;
        }
    }
}

/// `base[k] = v` for any key value (keyed store).
pub(crate) fn set_keyed(cx: &mut Cx, base: u64, k: u64, v: u64, strict: bool) -> Res<()> {
    if is_array(base)
        && let Some(i) = val(k).as_int32()
        && i >= 0
        && array_store(cx.rt(), base, i as u32, v)
    {
        return Ok(());
    }
    if base == UNDEFINED || base == NULL {
        let msg = format!(
            "Cannot set properties of {} (setting '{}')",
            describe(base),
            describe(k).trim_matches('"')
        );
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    let key = to_property_key(cx, k)?;
    set(cx, base, key, v, strict)
}

/// CreateDataPropertyOrThrow (§7.3.7) on an object: literals, `DefineNamedOwnProperty`,
/// `DefineKeyedOwnProperty`, `StaInArrayLiteral`, CopyDataProperties. The fast path takes a
/// new key on an extensible object and an existing configurable data property; everything
/// else (a routed kind always) goes through `[[DefineOwnProperty]]`, and a rejection throws a
/// TypeError. GC point.
pub(crate) fn define(cx: &mut Cx, o: u64, key: u64, v: u64) -> Res<()> {
    debug_assert!(is_object(o) && is_property_key(key));
    let rt = cx.rt();
    let key = intern_key(rt, key);
    let how = key::probe(key);
    if is_array(o)
        && let Some(i) = array_index_of(key)
        // An existing element is {w, e, c}; a new one needs an extensible array.
        && (object::array_get(o, i).is_some() || is_extensible(o))
        && object::array_set(rt, o, i, v)
    {
        return Ok(());
    }
    let exotic = exotic::own_attrs(rt, o, key, how);
    let stored = props::own_get_with(o, key, how);
    // A routed kind (ARGUMENTS, MODULE_NAMESPACE) takes its `[[DefineOwnProperty]]` hook.
    if exotic.is_none() && !exotic::is_routed(kind::cell_kind(o)) {
        match stored {
            None if is_extensible(o) => {
                props::own_add(rt, o, key, v, attr::DEFAULT);
                return Ok(());
            }
            Some((_, a)) if a & attr::CONFIGURABLE != 0 && a & attr::ACCESSOR == 0 => {
                props::own_define(rt, o, key, v, attr::DEFAULT);
                return Ok(());
            }
            _ => {}
        }
    }
    let exists = exotic.is_some() || stored.is_some();
    if define_own_property(cx, o, key, Desc::data(v, attr::DEFAULT))? {
        return Ok(());
    }
    let k = key_text(cx.rt(), key);
    let msg = if exists {
        format!("Cannot redefine property: {k}")
    } else {
        format!("Cannot define property {k}, object is not extensible")
    };
    Err(throw_error(cx, ErrorKind::TypeError, &msg))
}

/// Keyed define (CreateDataPropertyOrThrow) for any key value.
pub(crate) fn define_keyed(cx: &mut Cx, o: u64, k: u64, v: u64) -> Res<()> {
    if is_array(o)
        && let Some(i) = val(k).as_int32()
        && i >= 0
        && (object::array_get(o, i as u32).is_some() || is_extensible(o))
        && object::array_set(cx.rt(), o, i as u32, v)
    {
        return Ok(());
    }
    let key = to_property_key(cx, k)?;
    define(cx, o, key, v)
}

/// `delete base[k]` (§13.5.1.2, then `[[Delete]]` §10.1.10). Returns the boolean result.
pub(crate) fn delete(cx: &mut Cx, base: u64, k: u64, strict: bool) -> Res<bool> {
    if base == UNDEFINED || base == NULL {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Cannot convert undefined or null to object",
        ));
    }
    let key = to_property_key(cx, k)?;
    if !is_object(base) {
        // v0: primitives have no deletable own properties except string indices (false).
        return Ok(
            !(is_string(base) && (array_index_of(key).is_some() || is_atom(cx, key, Atom::Length)))
        );
    }
    let deleted = match exotic::delete(cx, base, key) {
        Some(r) => r?,
        None => ordinary_delete(cx.rt(), base, key),
    };
    if deleted {
        Ok(true)
    } else {
        deny(cx, key, strict)
    }
}

/// OrdinaryDelete (§10.1.10.1) of the property key `key` on the object `o`, past the
/// `[[Delete]]` hook of `o`: false for a non-configurable own property. GC point (a
/// dictionary move).
pub(crate) fn ordinary_delete(rt: &Runtime, o: u64, key: u64) -> bool {
    let how = key::probe(key);
    if is_array(o)
        && let Some(i) = array_index_of(key)
        && object::array_get(o, i).is_some()
    {
        // v0: a dense array keeps a hole.
        object::array_set(rt, o, i, crate::frame::HOLE);
        return true;
    }
    if let Some(a) = exotic::own_attrs(rt, o, key, how) {
        // Array `length`, a constructor's lazy `prototype` and String wrapper properties are
        // not configurable; elements were handled above.
        debug_assert!(a & attr::CONFIGURABLE == 0);
        return false;
    }
    match props::own_get_with(o, key, how) {
        Some((_, a)) if a & attr::CONFIGURABLE == 0 => false,
        Some(_) => {
            object::own_delete(rt, o, key);
            true
        }
        None => true,
    }
}

fn deny(cx: &mut Cx, key: u64, strict: bool) -> Res<bool> {
    if strict {
        let msg = format!(
            "Cannot delete property '{}' of object",
            key_text(cx.rt(), key)
        );
        Err(throw_error(cx, ErrorKind::TypeError, &msg))
    } else {
        Ok(false)
    }
}

/// HasProperty (§7.3.12) for the `in` operator: `k in o`.
pub(crate) fn has(cx: &mut Cx, o: u64, k: u64) -> Res<bool> {
    if !is_object(o) {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            &format!(
                "Cannot use 'in' operator to search for a key in {}",
                describe(o)
            ),
        ));
    }
    let key = to_property_key(cx, k)?;
    Ok(has_property(cx, o, key))
}

/// HasProperty(O, P) (§7.3.12, OrdinaryHasProperty along the chain) for a property key.
/// Runs no JS (no proxies yet) and allocates nothing.
pub(crate) fn has_property(cx: &Cx, o: u64, key: u64) -> bool {
    debug_assert!(is_object(o) && is_property_key(key));
    let how = key::probe(key);
    let mut cur = o;
    loop {
        if has_own_with(cx.rt(), cur, key, how) {
            return true;
        }
        let p = object::proto_of(cur);
        if p == NULL {
            return false;
        }
        cur = p;
    }
}

/// HasOwnProperty(O, P) (§7.3.13): `[[GetOwnProperty]]` is not undefined. Allocates nothing.
pub(crate) fn has_own_property(cx: &Cx, o: u64, key: u64) -> bool {
    debug_assert!(is_object(o) && is_property_key(key));
    has_own_with(cx.rt(), o, key, key::probe(key))
}

fn has_own_with(rt: &Runtime, o: u64, key: u64, how: Probe) -> bool {
    props::own_get_with(o, key, how).is_some() || exotic::own_attrs(rt, o, key, how).is_some()
}

/// InstanceofOperator(V, target) (§13.10.2) for `v instanceof f`:
///
/// ```text
/// f not an object ───────────────────────────────> TypeError
/// h = GetMethod(f, @@hasInstance)
///   h is %Function.prototype%[@@hasInstance] ────> OrdinaryHasInstance(f, v), no call
///   h is another function ───────────────────────> ToBoolean(Call(h, f, « v »))
///   h undefined/null: f not callable ────────────> TypeError
///                     else ──────────────────────> OrdinaryHasInstance(f, v)
/// ```
///
/// OrdinaryHasInstance of a bound function is InstanceofOperator(v, its target); the loop
/// takes that step in place, so a long bound chain costs no native stack (W4-BLTA-BOUND).
/// GC point (the `@@hasInstance` getter and handler run JS): `v` and `f` live in Rust locals.
pub(crate) fn instance_of(cx: &mut Cx, v: u64, f: u64) -> Res<bool> {
    let mut f = f;
    loop {
        // 1. If target is not an Object, throw a TypeError exception.
        if !is_object(f) {
            return Err(throw_error(
                cx,
                ErrorKind::TypeError,
                "Right-hand side of 'instanceof' is not an object",
            ));
        }
        // 2. Let instOfHandler be ? GetMethod(target, @@hasInstance).
        let key = symbol::well_known(cx.rt(), symbol::WellKnownSymbol::HasInstance);
        let h = get(cx, f, key)?;
        if h != UNDEFINED && h != NULL {
            if !is_callable(h) {
                let msg = format!("{} is not a function", describe(h));
                return Err(throw_error(cx, ErrorKind::TypeError, &msg));
            }
            // 3. Return ToBoolean(? Call(instOfHandler, target, « V »)). The intrinsic
            //    handler is OrdinaryHasInstance(this value, V): run it here.
            if !is_has_instance_intrinsic(cx.rt(), h) {
                let r = crate::call::call_function(cx, h, f, &[v])?;
                return Ok(crate::conv::to_boolean(r));
            }
            // OrdinaryHasInstance step 1: a non-callable C gives false.
            if !is_callable(f) {
                return Ok(false);
            }
        } else if !is_callable(f) {
            // 4. If IsCallable(target) is false, throw a TypeError exception.
            return Err(throw_error(
                cx,
                ErrorKind::TypeError,
                "Right-hand side of 'instanceof' is not callable",
            ));
        }
        // 5. Return ? OrdinaryHasInstance(target, V). Step 2: a bound function gives
        //    InstanceofOperator(V, [[BoundTargetFunction]]): the next round of the loop.
        if is_bound(f) {
            f = bound::target(f);
            continue;
        }
        return ordinary_has_instance_of_function(cx, f, v);
    }
}

/// OrdinaryHasInstance(C, O) (§7.3.21) for `Function.prototype[@@hasInstance]`
/// (`natives.rs`): false for a non-callable `c`; InstanceofOperator(O, target) for a bound
/// `c`; else the prototype chain walk. GC point: `c` and `o` live in Rust locals.
pub(crate) fn ordinary_has_instance(cx: &mut Cx, c: u64, o: u64) -> Res<bool> {
    // 1. If IsCallable(C) is false, return false.
    if !is_callable(c) {
        return Ok(false);
    }
    // 2. If C has a [[BoundTargetFunction]], return ? InstanceofOperator(O, BC).
    if is_bound(c) {
        return instance_of(cx, o, bound::target(c));
    }
    ordinary_has_instance_of_function(cx, c, o)
}

/// OrdinaryHasInstance(C, O) steps 3-7 for a callable `c` that is not bound.
fn ordinary_has_instance_of_function(cx: &mut Cx, c: u64, o: u64) -> Res<bool> {
    debug_assert!(is_callable(c) && !is_bound(c));
    // 3. If O is not an Object, return false.
    if !is_object(o) {
        return Ok(false);
    }
    // 4. Let P be ? Get(C, "prototype").
    let key = cx.rt().atom(Atom::Prototype);
    let proto = get(cx, c, key)?;
    // 5. If P is not an Object, throw a TypeError exception.
    if !is_object(proto) {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Function has non-object prototype in instanceof check",
        ));
    }
    // 6. Repeat: O = ? O.[[GetPrototypeOf]](); null gives false, P gives true.
    let mut o = object::proto_of(o);
    while o != NULL {
        if o == proto {
            return Ok(true);
        }
        o = object::proto_of(o);
    }
    Ok(false)
}

/// Whether `h` is `%Function.prototype%[@@hasInstance]` (VM builtin 0.30). Allocates nothing.
fn is_has_instance_intrinsic(rt: &Runtime, h: u64) -> bool {
    let id = crate::builtins::BuiltinId::new(
        crate::natives::VM_BLOCK,
        crate::natives::vm::FUNCTION_HAS_INSTANCE,
    );
    crate::builtins::existing_function(rt, id) == Some(h)
}

/// The keys a `for-in` loop visits (§14.7.5.9 EnumerateObjectProperties, v0: own and inherited
/// enumerable string keys, array and String wrapper indices first, no duplicates; symbols are
/// skipped), as a new array. A routed kind on the chain lists its keys through its
/// `[[OwnPropertyKeys]]` and `[[GetOwnProperty]]`. GC point.
pub(crate) fn for_in_keys(cx: &mut Cx, o: u64) -> Res<u64> {
    let rt = cx.rt();
    let keys = object::new_array(rt, 0);
    if !is_object(o) {
        if is_string(o) {
            for i in 0..object::str_len(o) {
                let s = crate::conv::number_string(cx, i as f64);
                object::array_set(rt, keys, i as u32, s);
            }
        }
        return Ok(keys);
    }
    // EnumerateObjectProperties (§14.7.5.9): each object's own keys in [[OwnPropertyKeys]]
    // order (§10.1.11: array indices ascending, then strings in creation order), then the
    // prototype's. A key seen before, enumerable or not, is skipped later.
    // GC: the Rust vectors hold only indices, key atoms of property storage (reachable from
    // `o`) and the `length` atom (a runtime root); a new index string goes into `keys` (a Rust
    // local, so a root) at once. A routed kind's keys come from its key array (`own`, a Rust
    // local kept live until they are pushed) and stay reachable from the object: namespace
    // export names from `nsf::EXPORTS`, arguments string keys from its storage.
    let mut seen_index: Vec<u32> = Vec::new();
    let mut seen_string: Vec<u64> = Vec::new();
    let mut cur = o;
    loop {
        // (index, enumerable) and (key, enumerable).
        let mut index_keys: Vec<(u32, bool)> = Vec::new();
        let mut string_keys: Vec<(u64, bool)> = Vec::new();
        // The key array of a routed kind, kept live until its keys are pushed below.
        let mut own = UNDEFINED;
        if exotic::is_routed(kind::cell_kind(cur)) {
            // A routed kind (ARGUMENTS, MODULE_NAMESPACE): its `[[OwnPropertyKeys]]` and
            // `[[GetOwnProperty]]` (the hooks, else the ordinary methods). GC points.
            own = own_property_keys(cx, cur);
            for j in 0..object::array_len(own) {
                let Some(k) = object::array_get(own, j) else {
                    continue;
                };
                if is_symbol_key(k) {
                    continue;
                }
                let Some((_, a, _)) = own_property(cx, cur, k)? else {
                    continue;
                };
                let enumerable = a & attr::ENUMERABLE != 0;
                match array_index_of(k) {
                    Some(n) => index_keys.push((n, enumerable)),
                    None => string_keys.push((k, enumerable)),
                }
            }
        } else {
            if is_array(cur) {
                for i in 0..object::array_len(cur) {
                    if object::array_get(cur, i).is_some() {
                        index_keys.push((i, true));
                    }
                }
            }
            // A String wrapper (§10.4.3.3): its indices come first, and its `length` (not
            // enumerable) first among the strings, so it hides an inherited `length`.
            if let Some(s) = wrapper::wrapper_value(cur).filter(|&p| is_string(p)) {
                index_keys.extend((0..object::str_len(s) as u32).map(|i| (i, true)));
                string_keys.push((rt.atom(Atom::Length), false));
            }
            for i in 0..object::own_count(cur) {
                let (k, _, a) = object::own_entry(cur, i);
                if is_symbol_key(k) {
                    continue;
                }
                let enumerable = a & attr::ENUMERABLE != 0;
                match array_index_of(k) {
                    Some(n) => index_keys.push((n, enumerable)),
                    None => string_keys.push((k, enumerable)),
                }
            }
        }
        index_keys.sort_by_key(|&(n, _)| n);
        for (n, enumerable) in index_keys {
            if seen_index.contains(&n) {
                continue;
            }
            seen_index.push(n);
            if enumerable {
                let s = crate::conv::number_string(cx, f64::from(n));
                let len = object::array_len(keys);
                object::array_set(cx.rt(), keys, len, s);
            }
        }
        for (k, enumerable) in string_keys {
            if seen_string.iter().any(|&x| object::key_eq(x, k)) {
                continue;
            }
            seen_string.push(k);
            if enumerable {
                let len = object::array_len(keys);
                object::array_set(cx.rt(), keys, len, k);
            }
        }
        std::hint::black_box(own);
        let p = object::proto_of(cur);
        if p == NULL {
            break;
        }
        cur = p;
    }
    Ok(keys)
}

// ---- [[GetOwnProperty]], [[DefineOwnProperty]], [[OwnPropertyKeys]] (builtin ABI, H20) ------

/// Where an own property lives, for the apply step of `[[DefineOwnProperty]]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OwnSlot {
    /// The object's property storage (`props.rs`: shape slots or the dictionary table).
    Storage,
    /// An exotic own property (`exotic.rs`).
    Exotic(Exotic),
}

/// The own property `key` of the object `o`: (value, attribute bits, where it lives). Array
/// `length` is handled by the callers. Only a routed kind's hook throws (§10.4.6.5 step 4).
/// GC point (a constructor's lazy `prototype` object, a routed hook).
fn own_property(cx: &mut Cx, o: u64, key: u64) -> Res<Option<(u64, i32, OwnSlot)>> {
    let how = key::probe(key);
    if let Some(r) = exotic::own_property(cx, o, key, how) {
        let (v, a, e) = r?;
        return Ok(Some((v, a, OwnSlot::Exotic(e))));
    }
    Ok(props::own_get_with(o, key, how).map(|(v, a)| (v, a, OwnSlot::Storage)))
}

/// `o.[[GetOwnProperty]](key)` (§10.1.5.1 OrdinaryGetOwnProperty, plus the exotic
/// properties of arrays, constructors and String wrappers; H20 R6). Only a routed kind's hook
/// throws (a namespace binding in its TDZ, §10.4.6.5). GC point.
pub(crate) fn get_own_property(cx: &mut Cx, o: u64, key: u64) -> Res<Option<Desc>> {
    debug_assert!(is_object(o) && is_property_key(key));
    if is_array(o) && is_atom(cx, key, Atom::Length) {
        // §10.4.2: {w: true, e: false, c: false}. v0: `length` is always writable.
        let len = num(f64::from(object::array_len(o)));
        return Ok(Some(Desc::data(len, attr::WRITABLE)));
    }
    Ok(own_property(cx, o, key)?.map(|(v, a, _)| accessor::stored_desc(v, a)))
}

/// IsExtensible (§7.2.5, ordinary objects): the shape's flag.
pub(crate) fn is_extensible(o: u64) -> bool {
    shape::is_extensible(shape::shape_of(o))
}

/// `o.[[PreventExtensions]]()` (§10.1.4.1). Always succeeds for ordinary objects. The
/// object moves to a non-extensible dictionary shape (OM-10). GC point.
pub(crate) fn prevent_extensions(rt: &Runtime, o: u64) -> bool {
    if is_extensible(o) {
        props::prevent_extensions(rt, o);
    }
    true
}

/// `o.[[SetPrototypeOf]](p)` (§10.1.2.1 OrdinarySetPrototypeOf; `p` is an object or null).
/// %Object.prototype% is an immutable prototype exotic object (§10.4.7). GC point.
pub(crate) fn set_prototype_of(rt: &Runtime, o: u64, p: u64) -> bool {
    debug_assert!(is_object(o) && (p == NULL || is_object(p)));
    if object::proto_of(o) == p {
        return true;
    }
    if !is_extensible(o) || o == rt.root(Root::ObjectPrototype) {
        return false;
    }
    object::set_proto(rt, o, p)
}

/// SameValue (§7.2.10).
pub(crate) fn same_value(a: u64, b: u64) -> bool {
    match (val(a).as_number(), val(b).as_number()) {
        (Some(x), Some(y)) => {
            (x.is_nan() && y.is_nan()) || (x == y && x.is_sign_negative() == y.is_sign_negative())
        }
        _ => crate::conv::strict_equals(a, b),
    }
}

/// `o.[[DefineOwnProperty]](key, desc)` (§10.1.6.1 OrdinaryDefineOwnProperty with
/// §10.1.6.3 ValidateAndApplyPropertyDescriptor; §10.4.2.1 for arrays). `Ok(false)` when the
/// definition is rejected. An accessor gets a new ACCESSOR_PAIR on every change (pairs are
/// immutable, `accessor.rs`). v0: a non-writable array `length`, and an array element that
/// would not be a data property {w, e, c}, throw "not implemented". A routed kind's hook
/// (`exotic::define_own_property`) runs first. GC point.
pub(crate) fn define_own_property(cx: &mut Cx, o: u64, key: u64, desc: Desc) -> Res<bool> {
    debug_assert!(is_object(o) && is_property_key(key));
    if let Some(r) = exotic::define_own_property(cx, o, key, desc) {
        return r;
    }
    ordinary_define_own_property(cx, o, key, desc)
}

/// [`define_own_property`] past the hook of `o`: OrdinaryDefineOwnProperty (with the array
/// and String wrapper parts). GC point.
pub(crate) fn ordinary_define_own_property(cx: &mut Cx, o: u64, key: u64, desc: Desc) -> Res<bool> {
    debug_assert!(is_object(o) && is_property_key(key));
    debug_assert!(
        !(desc.is_accessor() && desc.is_data()),
        "ToPropertyDescriptor rejects a descriptor with get or set and value or writable"
    );
    if is_array(o) && is_atom(cx, key, Atom::Length) {
        // ArraySetLength step 1 (no [[Value]]): OrdinaryDefineOwnProperty against the data
        // property {w: true, e: false, c: false}, which rejects an accessor (step 5.c).
        if desc.is_accessor() {
            return Ok(false);
        }
        return array_define_length(cx, o, desc);
    }
    let Some((cur_v, cur_a, slot)) = own_property(cx, o, key)? else {
        // §10.1.6.3 step 2: a new property; absent fields default to undefined / false.
        if !is_extensible(o) {
            return Ok(false);
        }
        if desc.is_accessor() {
            let get = desc.get.unwrap_or(UNDEFINED);
            let set = desc.set.unwrap_or(UNDEFINED);
            return add_accessor(cx, o, key, get, set, desc.new_attrs());
        }
        let v = desc.value.unwrap_or(UNDEFINED);
        return add_own_property(cx, o, key, v, desc.new_attrs());
    };
    let current = accessor::stored_desc(cur_v, cur_a);
    // Steps 4-5.
    if !validate(&desc, &current) {
        return Ok(false);
    }
    // Step 6: the new value slot (a data value or a pair) and attributes.
    let ec = desc.merge_attrs(cur_a) & (attr::ENUMERABLE | attr::CONFIGURABLE);
    let (v, attrs) = if desc.is_accessor() || (current.is_accessor() && !desc.is_data()) {
        // 6.a (data to accessor; absent halves are undefined) or 6.c (an accessor keeps the
        // halves the descriptor does not name).
        let keep = |d: Option<u64>, c: Option<u64>| d.or(c).unwrap_or(UNDEFINED);
        let get = keep(desc.get, current.get);
        let set = keep(desc.set, current.set);
        let attrs = attr::ACCESSOR | ec;
        if current.is_accessor()
            && attrs == cur_a
            && get == current.get.unwrap_or(UNDEFINED)
            && set == current.set.unwrap_or(UNDEFINED)
        {
            return Ok(true);
        }
        if let OwnSlot::Exotic(_) = slot {
            // Exotic own properties are data properties; only a dense element gets here
            // (a String wrapper's are not configurable).
            return Err(not_implemented(cx, "accessor array elements"));
        }
        // GC point: `o`, `key`, `get` and `set` live in Rust locals.
        (accessor::new_pair(cx.rt(), get, set), attrs)
    } else if current.is_accessor() {
        // 6.b: accessor to data; absent fields are undefined / false.
        let w = if desc.writable == Some(true) {
            attr::WRITABLE
        } else {
            0
        };
        (desc.value.unwrap_or(UNDEFINED), w | ec)
    } else {
        // 6.c: data to data.
        (desc.value.unwrap_or(cur_v), desc.merge_attrs(cur_a))
    };
    if attrs == cur_a && v == cur_v {
        return Ok(true);
    }
    let rt = cx.rt();
    match slot {
        // New attributes move a fast object to dictionary mode (OM-10, props.rs).
        OwnSlot::Storage => props::own_define(rt, o, key, v, attrs),
        OwnSlot::Exotic(Exotic::Element(i)) => {
            if attrs != attr::DEFAULT {
                return Err(not_implemented(
                    cx,
                    "array elements that are not writable, enumerable and configurable",
                ));
            }
            object::array_set(rt, o, i, v);
        }
        // Validation passed on a non-writable, non-configurable property: nothing changes
        // (SameValue held for the value).
        OwnSlot::Exotic(Exotic::Wrapper) => {}
        // A routed kind's own property: its `[[DefineOwnProperty]]` hook applies changes.
        OwnSlot::Exotic(Exotic::Routed) => {
            return Err(not_implemented(
                cx,
                "an ordinary define on a routed exotic property",
            ));
        }
    }
    Ok(true)
}

/// ValidateAndApplyPropertyDescriptor (§10.1.6.3) steps 4-5: may `desc` change the existing
/// property `current` (a complete data or accessor descriptor)?
fn validate(desc: &Desc, current: &Desc) -> bool {
    if current.configurable == Some(true) {
        return true;
    }
    // 5.a-b.
    if desc.configurable == Some(true) {
        return false;
    }
    if desc
        .enumerable
        .is_some_and(|e| Some(e) != current.enumerable)
    {
        return false;
    }
    // 5.c: a non-generic descriptor must keep the kind.
    let generic = !desc.is_accessor() && !desc.is_data();
    if !generic && desc.is_accessor() != current.is_accessor() {
        return false;
    }
    let differs =
        |d: Option<u64>, c: Option<u64>| d.is_some_and(|d| !same_value(d, c.unwrap_or(UNDEFINED)));
    if current.is_accessor() {
        // 5.d.
        return !differs(desc.get, current.get) && !differs(desc.set, current.set);
    }
    // 5.e.
    if current.writable == Some(false) {
        if desc.writable == Some(true) {
            return false;
        }
        if differs(desc.value, current.value) {
            return false;
        }
    }
    true
}

/// Adds the new own accessor property `key` with `get`, `set` (callables or undefined) and
/// the enumerable and configurable bits of `attrs` (the object is extensible). GC point.
fn add_accessor(cx: &mut Cx, o: u64, key: u64, get: u64, set: u64, attrs: i32) -> Res<bool> {
    if is_array(o) && array_index_of(key).is_some() {
        return Err(not_implemented(cx, "accessor array elements"));
    }
    let rt = cx.rt();
    // GC point: `o`, `key` and the pair live in Rust locals.
    let pair = accessor::new_pair(rt, get, set);
    let attrs = attr::ACCESSOR | (attrs & (attr::ENUMERABLE | attr::CONFIGURABLE));
    props::own_define(rt, o, key, pair, attrs);
    Ok(true)
}

/// Adds the new own property `key` (the object is extensible). GC point.
fn add_own_property(cx: &mut Cx, o: u64, key: u64, v: u64, attrs: i32) -> Res<bool> {
    let rt = cx.rt();
    if is_array(o)
        && let Some(i) = array_index_of(key)
    {
        if attrs != attr::DEFAULT {
            return Err(not_implemented(
                cx,
                "array elements that are not writable, enumerable and configurable",
            ));
        }
        // §10.4.2.1 step 3: an index at or past `length` raises it (array_set does).
        if object::array_set(rt, o, i, v) {
            return Ok(true);
        }
        // Past the v0 dense limit the index becomes a dictionary property and `length`
        // stays (H18 v0 notes), as in [[Set]] and literals.
    }
    props::own_define(rt, o, key, v, attrs);
    Ok(true)
}

/// ArraySetLength (§10.4.2.4) for `[[DefineOwnProperty]]("length", desc)`. v0: `length`
/// stays writable.
fn array_define_length(cx: &mut Cx, a: u64, desc: Desc) -> Res<bool> {
    let rt = cx.rt();
    let new_len = match desc.value {
        Some(v) => {
            let n = to_uint32_of(cx, v)?;
            let d = crate::conv::to_number(cx, v)?;
            if f64::from(n) != d {
                return Err(throw_error(
                    cx,
                    ErrorKind::RangeError,
                    "Invalid array length",
                ));
            }
            Some(n)
        }
        None => None,
    };
    // The current descriptor is {w: true, e: false, c: false}.
    if desc.configurable == Some(true) || desc.enumerable == Some(true) {
        return Ok(false);
    }
    if desc.writable == Some(false) {
        return Err(not_implemented(cx, "a non-writable array length"));
    }
    if let Some(n) = new_len
        && !object::array_set_length(rt, a, n)
    {
        return Err(throw_error(
            cx,
            ErrorKind::RangeError,
            "Invalid array length",
        ));
    }
    Ok(true)
}

/// `o.[[OwnPropertyKeys]]()` (§10.1.11.1 OrdinaryOwnPropertyKeys, §10.4.2 arrays, §10.4.3.3
/// String wrappers) as a new array: array indices ascending, then strings in creation order,
/// then symbols in creation order. The exotic properties come where the spec creates them:
/// `length` of an array or String wrapper first among the strings; a constructor's lazy
/// `prototype` after its `length` and `name`. A routed kind's hook
/// (`exotic::own_property_keys`) runs first. GC point.
pub(crate) fn own_property_keys(cx: &mut Cx, o: u64) -> u64 {
    debug_assert!(is_object(o));
    if let Some(keys) = exotic::own_property_keys(cx, o) {
        return keys;
    }
    ordinary_own_property_keys(cx, o)
}

/// [`own_property_keys`] past the hook of `o`: OrdinaryOwnPropertyKeys (with the array and
/// String wrapper parts). GC point.
pub(crate) fn ordinary_own_property_keys(cx: &mut Cx, o: u64) -> u64 {
    debug_assert!(is_object(o));
    let rt = cx.rt();
    // GC: `keys` is a Rust local (a root by the stack scan); the stored keys stay reachable
    // from `o`, and no JS runs here, so the storage does not change under the loops.
    let keys = object::new_array(rt, 0);
    let push = |k: u64| {
        let n = object::array_len(keys);
        object::array_set(rt, keys, n, k);
    };
    // 1. Integer indices ascending: elements, String wrapper indices, stored index keys.
    let mut indices: Vec<u32> = Vec::new();
    if is_array(o) {
        indices.extend((0..object::array_len(o)).filter(|&i| object::array_get(o, i).is_some()));
    }
    if let Some(s) = wrapper::wrapper_value(o).filter(|&p| is_string(p)) {
        indices.extend(0..object::str_len(s) as u32);
    }
    let stored = |i: usize| object::own_entry(o, i).0;
    indices.extend((0..object::own_count(o)).filter_map(|i| array_index_of(stored(i))));
    indices.sort_unstable();
    indices.dedup();
    for i in indices {
        // The atom (W4-KEY-EQ): callers compare and look up with these keys.
        push(crate::key::index_key(rt, i));
    }
    // 2. Strings in creation order: an exotic `length` first, a lazy `prototype` last (no
    // stored key is younger than it, `exotic::lazy_prototype_key`).
    if let Some(k) = exotic::virtual_length_key(rt, o) {
        push(k);
    }
    for i in 0..object::own_count(o) {
        let k = stored(i);
        if !is_symbol_key(k) && array_index_of(k).is_none() {
            push(k);
        }
    }
    if let Some(p) = exotic::lazy_prototype_key(rt, o) {
        push(p);
    }
    // 3. Symbols in creation order. A private name is not a property key (§6.2.12): it is
    // never listed.
    for i in 0..object::own_count(o) {
        let k = stored(i);
        if is_symbol_key(k) && !crate::symbol::is_private(k) {
            push(k);
        }
    }
    keys
}

// ---- integer-indexed access without a key (builtin ABI H20 ABI-1, `abi/index.rs`) -----------
//
// `Get`, `HasProperty`, `Set` and `[[Delete]]` with the key `! ToString(𝔽(i))` for an integer
// `i` (a builtin's loop index, up to 2^53 - 1). A dense element and a missing index need no key,
// so these paths make no atom for them (the key path interns one per new index text).
//
// ```text
// i ≤ 2^53 - 1: find_index walks the chain from o, per object own_index(i) without the key:
//   routed kind (ARGUMENTS, MODULE_NAMESPACE)    -> Unknown: its hooks, with the key
//   array element / String wrapper index        -> found
//   array-index i: fast shape                    -> absent (an array-index key makes a
//                                                   dictionary, OM-10)
//   stored keys ≤ INDEX_SCAN_MAX                 -> scan them: array_index_of, or the digits
//   more stored keys                             -> Unknown: the key path from that object
//   array-index i and no proto_index_keys        -> the walk stops after the receiver
// i > 2^53 - 1, or Unknown: the key (number_key, an atom) and the ordinary path
// ```

/// The largest own-property count that [`own_index`] scans for a stored integer key; an object
/// with more takes the key path (an atom, found in the table when the key is stored).
const INDEX_SCAN_MAX: usize = 64;

/// The largest integer whose decimal digits are `ToString(𝔽(i))` (2^53 - 1): above it, `𝔽(i)`
/// rounds and the key path makes the key.
const MAX_SAFE_INDEX: u64 = (1 << 53) - 1;

/// Where the own property `! ToString(𝔽(i))` of an object is, found without the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OwnIndex {
    /// A dense element or a stored property: its value (the ACCESSOR_PAIR of an accessor) and
    /// attribute bits.
    Stored(u64, i32),
    /// Index `i` of a String wrapper: this string (§10.4.3, {e}).
    WrapperChar(u64),
    /// The object has no such own property.
    Absent,
    /// Too many stored keys to scan, or a routed kind (ARGUMENTS, MODULE_NAMESPACE) whose
    /// hooks decide: ask this object with the key.
    Unknown,
}

/// The array index `i` names (`i < 2^32 - 1`), if any.
fn array_index_u64(i: u64) -> Option<u32> {
    u32::try_from(i).ok().filter(|&n| n != u32::MAX)
}

/// The decimal digits of `i` in `buf`, as a slice of it: `ToString(𝔽(i))` for
/// `i ≤ 2^53 - 1` (Number::toString writes an integer below 10^21 as its digits).
fn index_digits(i: u64, buf: &mut [u8; 20]) -> &[u8] {
    let mut at = buf.len();
    let mut n = i;
    loop {
        at -= 1;
        buf[at] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    &buf[at..]
}

/// The own property `! ToString(𝔽(i))` of the object `o` (`text` is that string), found
/// without the key: the exotic ones of `exotic.rs` that an integer key can name (array
/// elements, String wrapper indices), then the stored ones. A routed kind (ARGUMENTS,
/// MODULE_NAMESPACE) always answers `Unknown`: its hooks decide, with the key. A new exotic
/// kind with integer keys adds its case here too. Not a GC point.
fn own_index(o: u64, i: u64, text: &[u8]) -> OwnIndex {
    let k = kind::cell_kind(o);
    if exotic::is_routed(k) {
        return OwnIndex::Unknown;
    }
    let index = array_index_u64(i);
    if let Some(n) = index {
        if k == kind::ARRAY.get() {
            if let Some(v) = object::array_get(o, n) {
                return OwnIndex::Stored(v, attr::DEFAULT);
            }
        } else if k == kind::PRIMITIVE_WRAPPER.get()
            && let Some(s) = wrapper::wrapper_value(o).filter(|&p| is_string(p))
            && (n as usize) < object::str_len(s)
        {
            return OwnIndex::WrapperChar(s);
        }
        // An array-index key moves a fast object to dictionary mode (OM-10, `shape::add`).
        if !shape::is_dictionary(shape::shape_of(o)) {
            return OwnIndex::Absent;
        }
    }
    let count = object::own_count(o);
    if count > INDEX_SCAN_MAX {
        return OwnIndex::Unknown;
    }
    for j in 0..count {
        let (key, v, a) = object::own_entry(o, j);
        // Stored keys are atoms (`props::own_add`): an array index answers from its hash
        // field; another integer key compares its digits.
        let hit = match index {
            Some(n) => array_index_of(key) == Some(n),
            None => is_string(key) && string::equals_text(key, string::JsStr::Latin1(text)),
        };
        if hit {
            return OwnIndex::Stored(v, a);
        }
    }
    OwnIndex::Absent
}

/// The first object on the prototype chain from `o` that has the own property
/// `! ToString(𝔽(i))` (`text`), and where it is; `Absent` if none has it, `Unknown` if that
/// object must be asked with the key. While no prototype has an array-index property
/// (`shape::proto_index_keys`), an array index stops at the receiver. Not a GC point.
fn find_index(rt: &Runtime, o: u64, i: u64, text: &[u8]) -> (u64, OwnIndex) {
    let array_index = array_index_u64(i).is_some();
    let mut cur = o;
    loop {
        let w = own_index(cur, i, text);
        if w != OwnIndex::Absent || (array_index && !shape::proto_index_keys(rt)) {
            return (cur, w);
        }
        cur = object::proto_of(cur);
        if cur == NULL {
            return (o, OwnIndex::Absent);
        }
    }
}

/// The key `! ToString(𝔽(i))` (an atom). GC point when the atom is new.
fn index_key_of(rt: &Runtime, i: u64) -> u64 {
    key::number_key(rt, i as f64)
}

/// HasProperty(O, ! ToString(𝔽(i))) (§7.3.12) on the object `o`. No atom for an element or a
/// missing index. GC point only on the key path.
pub(crate) fn has_index(cx: &Cx, o: u64, i: u64) -> bool {
    debug_assert!(is_object(o));
    if i <= MAX_SAFE_INDEX {
        let mut buf = [0u8; 20];
        let text = index_digits(i, &mut buf);
        match find_index(cx.rt(), o, i, text) {
            (_, OwnIndex::Stored(..) | OwnIndex::WrapperChar(_)) => return true,
            (_, OwnIndex::Absent) => return false,
            (cur, OwnIndex::Unknown) => {
                let key = index_key_of(cx.rt(), i);
                return has_property(cx, cur, key);
            }
        }
    }
    let key = index_key_of(cx.rt(), i);
    has_property(cx, o, key)
}

/// Get(O, ! ToString(𝔽(i))) (§7.3.2, OrdinaryGet) on the object `o`. No atom for an element
/// or a missing index. GC point (a getter, a String wrapper's character, the key path).
pub(crate) fn get_index(cx: &mut Cx, o: u64, i: u64) -> Res<u64> {
    debug_assert!(is_object(o));
    if i <= MAX_SAFE_INDEX {
        let mut buf = [0u8; 20];
        let text = index_digits(i, &mut buf);
        match find_index(cx.rt(), o, i, text) {
            (_, OwnIndex::Stored(v, a)) if a & attr::ACCESSOR == 0 => return Ok(v),
            (_, OwnIndex::Stored(pair, _)) => return accessor::call_getter(cx, pair, o),
            (_, OwnIndex::WrapperChar(s)) => return Ok(string::char_at(cx.rt(), s, i as usize)),
            (_, OwnIndex::Absent) => return Ok(UNDEFINED),
            (cur, OwnIndex::Unknown) => {
                // GC point: `cur` and `o` live in Rust locals.
                let key = index_key_of(cx.rt(), i);
                return get_with_receiver(cx, cur, key, o);
            }
        }
    }
    let key = index_key_of(cx.rt(), i);
    get_from(cx, o, key)
}

/// `o.[[Set]](! ToString(𝔽(i)), v, o)` (§10.1.9 OrdinarySet) on the object `o`: `Ok(false)`
/// when the assignment fails. An array element of a dense array is stored without a key
/// (`array_store`, as a keyed store does); everything else takes the key path. GC point.
pub(crate) fn set_index(cx: &mut Cx, o: u64, i: u64, v: u64) -> Res<bool> {
    debug_assert!(is_object(o));
    if is_array(o)
        && let Some(n) = array_index_u64(i)
        && array_store(cx.rt(), o, n, v)
    {
        return Ok(true);
    }
    // GC point: `o` and `v` live in Rust locals.
    let key = index_key_of(cx.rt(), i);
    ordinary_set(cx, o, key, v)
}

/// `o.[[Delete]](! ToString(𝔽(i)))` (§10.1.10) on the object `o`, with [`delete`]'s `strict`
/// (a TypeError instead of `false`). An element becomes a hole and a missing index answers
/// true, without a key. GC point.
pub(crate) fn delete_index(cx: &mut Cx, o: u64, i: u64, strict: bool) -> Res<bool> {
    debug_assert!(is_object(o));
    if i <= MAX_SAFE_INDEX {
        if is_array(o)
            && let Some(n) = array_index_u64(i)
            && object::array_get(o, n).is_some()
        {
            // v0: a dense array keeps a hole (as `delete`).
            object::array_set(cx.rt(), o, n, crate::frame::HOLE);
            return Ok(true);
        }
        let mut buf = [0u8; 20];
        if own_index(o, i, index_digits(i, &mut buf)) == OwnIndex::Absent {
            return Ok(true);
        }
    }
    let key = index_key_of(cx.rt(), i);
    delete(cx, o, key, strict)
}
