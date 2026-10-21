//! Property access on v0 objects: [[Get]], [[Set]], [[DefineOwnProperty]], [[Delete]],
//! [[HasProperty]], [[GetOwnProperty]] and [[OwnPropertyKeys]] along the prototype chain
//! (§10.1). v0: data properties only (no accessors), string keys only (no symbols), dense
//! array elements only.
//!
//! | Receiver | Own properties seen before the table |
//! |---|---|
//! | array | `length` {w}, array-index keys (elements, {w, e, c}) |
//! | function | `prototype` (created on first access for constructors), `length` {c}, `name` {c} |
//! | primitive wrapper | `wrapper::string_own_property` (String wrappers: `length`, indices) |
//! | string primitive | `length`, array-index keys; then `String.prototype` |
//! | number / boolean primitive | `Number.prototype` / `Boolean.prototype` |
//!
//! The builtin ABI (H20 ObjectOps, abi/mod.rs) calls [`get_own_property`],
//! [`define_own_property`] (ValidateAndApplyPropertyDescriptor) and [`own_property_keys`].

use crate::conv::{Res, describe, num, to_property_key, to_uint32_of};
use crate::descriptor::Desc;
use crate::error::{not_implemented, throw_error};
use crate::frame::{NULL, UNDEFINED, int32_bits};
use crate::interp::Cx;
use crate::key::{array_index_of, intern_key, is_property_key, is_symbol_key, key_text};
use crate::kinds::{is_callable, is_wrapper};
use crate::object::{
    self, attr, is_array, is_function, is_object, is_string, obj, own_define, own_get,
};
use crate::runtime::{Atom, ErrorKind, Root, Runtime, val};
use crate::{string, wrapper};

fn is_atom(cx: &Cx, key: u64, a: Atom) -> bool {
    object::key_eq(key, cx.rt().atom(a))
}

/// The lazily created `prototype` object of a constructor function (v0: created on first
/// access, as V8 does for most functions).
fn function_prototype_property(cx: &mut Cx, f: u64) -> u64 {
    let rt = cx.rt();
    let key = rt.atom(Atom::Prototype);
    if let Some((v, _)) = own_get(f, key) {
        return v;
    }
    let p = object::new_plain_object(rt);
    own_define(rt, p, rt.atom(Atom::Constructor), f, attr::HIDDEN);
    own_define(rt, f, key, p, attr::WRITABLE);
    p
}

/// A virtual own property of `o` (not in its table), if any.
fn virtual_get(cx: &mut Cx, o: u64, key: u64) -> Option<u64> {
    if is_array(o) {
        if is_atom(cx, key, Atom::Length) {
            return Some(num(f64::from(object::array_len(o))));
        }
        if let Some(i) = array_index_of(key) {
            return object::array_get(o, i);
        }
    } else if is_function(o) {
        let fi = object::function_info(o);
        if fi.is_constructor() && is_atom(cx, key, Atom::Prototype) {
            return Some(function_prototype_property(cx, o));
        }
        if own_get(o, key).is_none() {
            if is_atom(cx, key, Atom::Length) {
                return Some(num(f64::from(fi.length)));
            }
            if is_atom(cx, key, Atom::Name) {
                let name = fi.name.clone();
                return Some(object::new_string(cx.rt(), &name));
            }
        }
    }
    None
}

/// OrdinaryGet over the prototype chain, starting at object `o`.
pub(crate) fn get_from(cx: &mut Cx, mut o: u64, key: u64) -> Res<u64> {
    loop {
        if let Some(v) = virtual_get(cx, o, key) {
            return Ok(v);
        }
        if let Some((v, _)) = own_get(o, key) {
            return Ok(v);
        }
        let p = object::proto_of(o);
        if p == NULL {
            return Ok(UNDEFINED);
        }
        o = p;
    }
}

/// `base[key]` with a property-key string (GetValue on a property reference).
pub(crate) fn get(cx: &mut Cx, base: u64, key: u64) -> Res<u64> {
    debug_assert!(is_property_key(key));
    if is_object(base) {
        return get_from(cx, base, key);
    }
    let rt = cx.rt();
    let proto = if is_string(base) {
        if is_atom(cx, key, Atom::Length) {
            return Ok(num(object::str_len(base) as f64));
        }
        if let Some(i) = array_index_of(key)
            && (i as usize) < object::str_len(base)
        {
            return Ok(string::char_at(rt, base, i as usize));
        }
        rt.root(Root::StringPrototype)
    } else if val(base).is_number() {
        rt.root(Root::NumberPrototype)
    } else if base == object_bool(true) || base == object_bool(false) {
        rt.root(Root::BooleanPrototype)
    } else {
        let k = key_text(rt, key);
        let msg = format!(
            "Cannot read properties of {} (reading '{k}')",
            describe(base)
        );
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    };
    get_from(cx, proto, key)
}

#[inline]
fn object_bool(b: bool) -> u64 {
    crate::conv::boolean(b)
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

/// OrdinarySet (§10.1.9) with receiver = `base`. v0: no accessors; a non-writable own or
/// inherited property makes the assignment fail (TypeError in strict code).
pub(crate) fn set(cx: &mut Cx, base: u64, key: u64, v: u64, strict: bool) -> Res<()> {
    debug_assert!(is_property_key(key));
    if !is_object(base) {
        if base == UNDEFINED || base == NULL {
            let msg = format!(
                "Cannot set properties of {} (setting '{}')",
                describe(base),
                key_text(cx.rt(), key)
            );
            return Err(throw_error(cx, ErrorKind::TypeError, &msg));
        }
        // Primitive receiver: v0 drops the store (sloppy) or throws (strict).
        if strict {
            let msg = format!(
                "Cannot create property '{}' on {}",
                key_text(cx.rt(), key),
                describe(base)
            );
            return Err(throw_error(cx, ErrorKind::TypeError, &msg));
        }
        return Ok(());
    }
    if ordinary_set(cx, base, key, v)? {
        Ok(())
    } else {
        fail(cx, key, strict)
    }
}

/// `o.[[Set]](key, v, o)` (§10.1.9 OrdinarySet, receiver = `o`) on an object. `Ok(false)`
/// when the assignment fails (a non-writable property on the chain, or a new property on a
/// non-extensible object). v0: no accessors.
pub(crate) fn ordinary_set(cx: &mut Cx, o: u64, key: u64, v: u64) -> Res<bool> {
    debug_assert!(is_object(o) && is_property_key(key));
    let rt = cx.rt();
    if is_array(o) {
        if is_atom(cx, key, Atom::Length) {
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
    }
    if is_function(o) && object::function_info(o).is_constructor() {
        // Materialize `prototype` so the store below replaces it.
        if is_atom(cx, key, Atom::Prototype) {
            function_prototype_property(cx, o);
        }
    }
    // Find the property on the chain to check writability.
    let mut cur = o;
    loop {
        if let Some((_, a)) = own_get(cur, key) {
            if a & attr::WRITABLE == 0 {
                return Ok(false);
            }
            break;
        }
        let p = object::proto_of(cur);
        if p == NULL {
            break;
        }
        cur = p;
    }
    if object::own_replace(rt, o, key, v) {
        return Ok(true);
    }
    if !is_extensible(o) {
        return Ok(false);
    }
    own_define(rt, o, key, v, attr::DEFAULT);
    Ok(true)
}

/// Stores element `i` of the array `a` unless that adds an element to a non-extensible
/// array. False if it stored nothing (also past the v0 dense limit).
fn array_store(rt: &Runtime, a: u64, i: u32, v: u64) -> bool {
    if object::array_get(a, i).is_none() && !is_extensible(a) {
        return false;
    }
    object::array_set(rt, a, i, v)
}

fn fail(cx: &mut Cx, key: u64, strict: bool) -> Res<()> {
    if strict {
        let msg = format!(
            "Cannot assign to read only property '{}' of object",
            key_text(cx.rt(), key)
        );
        Err(throw_error(cx, ErrorKind::TypeError, &msg))
    } else {
        Ok(())
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

/// CreateDataPropertyOrThrow on an object (literals, `DefineNamedOwnProperty`).
pub(crate) fn define(cx: &mut Cx, o: u64, key: u64, v: u64) -> Res<()> {
    debug_assert!(is_object(o) && is_property_key(key));
    let rt = cx.rt();
    if is_array(o) {
        if let Some(i) = array_index_of(key)
            && object::array_set(rt, o, i, v)
        {
            return Ok(());
        }
        if is_atom(cx, key, Atom::Length) {
            return set(cx, o, key, v, true);
        }
    }
    own_define(rt, o, key, v, attr::DEFAULT);
    Ok(())
}

/// Keyed define for any key value.
pub(crate) fn define_keyed(cx: &mut Cx, o: u64, k: u64, v: u64) -> Res<()> {
    if is_array(o)
        && let Some(i) = val(k).as_int32()
        && i >= 0
        && object::array_set(cx.rt(), o, i as u32, v)
    {
        return Ok(());
    }
    let key = to_property_key(cx, k)?;
    define(cx, o, key, v)
}

/// `delete base[k]` (§13.5.1.2). Returns the boolean result.
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
    if is_array(base) {
        if is_atom(cx, key, Atom::Length) {
            return deny(cx, key, strict);
        }
        if let Some(i) = array_index_of(key) {
            if i < object::array_len(base) {
                // v0: a dense array keeps a hole.
                object::array_set(cx.rt(), base, i, crate::frame::HOLE);
            }
            return Ok(true);
        }
    }
    match own_get(base, key) {
        Some((_, a)) if a & attr::CONFIGURABLE == 0 => deny(cx, key, strict),
        Some(_) => {
            object::own_delete(cx.rt(), base, key);
            Ok(true)
        }
        None => Ok(true),
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
    let mut cur = o;
    loop {
        if has_own_property(cx, cur, key) {
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
    if own_get(o, key).is_some() {
        return true;
    }
    if is_array(o) {
        is_atom(cx, key, Atom::Length)
            || array_index_of(key).is_some_and(|i| object::array_get(o, i).is_some())
    } else if is_function(o) {
        is_atom(cx, key, Atom::Length)
            || is_atom(cx, key, Atom::Name)
            || object::function_info(o).is_constructor() && is_atom(cx, key, Atom::Prototype)
    } else if is_wrapper(o) {
        wrapper::string_own_property(cx.rt(), o, key).is_some()
    } else {
        false
    }
}

/// OrdinaryHasInstance (§7.3.22) for `v instanceof f`. v0: no `@@hasInstance`, no bound
/// functions.
pub(crate) fn instance_of(cx: &mut Cx, v: u64, f: u64) -> Res<bool> {
    if !is_callable(f) {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Right-hand side of 'instanceof' is not callable",
        ));
    }
    if !is_object(v) {
        return Ok(false);
    }
    let key = cx.rt().atom(Atom::Prototype);
    let proto = get(cx, f, key)?;
    if !is_object(proto) {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Function has non-object prototype in instanceof check",
        ));
    }
    let mut o = object::proto_of(v);
    while o != NULL {
        if o == proto {
            return Ok(true);
        }
        o = object::proto_of(o);
    }
    Ok(false)
}

/// The keys a `for-in` loop visits (§14.7.5.9 EnumerateObjectProperties, v0: own and inherited
/// enumerable string keys, array indices first, no duplicates), as a new array.
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
    // GC: the Rust vectors hold only indices and key strings of property tables (reachable
    // from `o`); a new index string goes into `keys` (a Rust local, so a root) at once.
    let mut seen_index: Vec<u32> = Vec::new();
    let mut seen_string: Vec<u64> = Vec::new();
    let mut cur = o;
    loop {
        // (index, enumerable) and (key, enumerable).
        let mut index_keys: Vec<(u32, bool)> = Vec::new();
        let mut string_keys: Vec<(u64, bool)> = Vec::new();
        if is_array(cur) {
            for i in 0..object::array_len(cur) {
                if object::array_get(cur, i).is_some() {
                    index_keys.push((i, true));
                }
            }
        }
        for i in 0..object::own_count(cur) {
            let (k, _, a) = object::own_entry(cur, i);
            let enumerable = a & attr::ENUMERABLE != 0;
            match array_index_of(k) {
                Some(n) => index_keys.push((n, enumerable)),
                None => string_keys.push((k, enumerable)),
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
        let p = object::proto_of(cur);
        if p == NULL {
            break;
        }
        cur = p;
    }
    Ok(keys)
}

// ---- [[GetOwnProperty]], [[DefineOwnProperty]], [[OwnPropertyKeys]] (builtin ABI, H20) ------

/// Where an own property lives, for the apply step of [[DefineOwnProperty]].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OwnSlot {
    /// The property table (`props.rs`).
    Table,
    /// A dense array element.
    Element(u32),
    /// A function's `length` or `name` that is not in its table yet.
    FunctionVirtual,
    /// A String wrapper's `length` or index (never writable or configurable).
    WrapperVirtual,
}

/// The own property `key` of the object `o`: (value, attribute bits, where it lives). Array
/// `length` is handled by the callers. GC point (a function's `name` string, a constructor's
/// lazy `prototype` object).
fn own_property(cx: &mut Cx, o: u64, key: u64) -> Option<(u64, i32, OwnSlot)> {
    if is_array(o) {
        if let Some(i) = array_index_of(key)
            && let Some(v) = object::array_get(o, i)
        {
            return Some((v, attr::DEFAULT, OwnSlot::Element(i)));
        }
    } else if is_function(o) {
        let fi = object::function_info(o);
        if fi.is_constructor() && is_atom(cx, key, Atom::Prototype) {
            function_prototype_property(cx, o);
        } else if own_get(o, key).is_none() {
            // §10.2.9 SetFunctionName and §10.2.10 SetFunctionLength: {w: false, e: false,
            // c: true}.
            if is_atom(cx, key, Atom::Length) {
                let v = num(f64::from(fi.length));
                return Some((v, attr::CONFIGURABLE, OwnSlot::FunctionVirtual));
            }
            if is_atom(cx, key, Atom::Name) {
                let name = fi.name.clone();
                let v = object::new_string(cx.rt(), &name);
                return Some((v, attr::CONFIGURABLE, OwnSlot::FunctionVirtual));
            }
        }
    } else if is_wrapper(o)
        && let Some((v, a)) = wrapper::string_own_property(cx.rt(), o, key)
    {
        return Some((v, a, OwnSlot::WrapperVirtual));
    }
    own_get(o, key).map(|(v, a)| (v, a, OwnSlot::Table))
}

/// `o.[[GetOwnProperty]](key)` (§10.1.5.1 OrdinaryGetOwnProperty, plus the virtual
/// properties of arrays, functions and String wrappers; H20 R6). GC point.
pub(crate) fn get_own_property(cx: &mut Cx, o: u64, key: u64) -> Option<Desc> {
    debug_assert!(is_object(o) && is_property_key(key));
    if is_array(o) && is_atom(cx, key, Atom::Length) {
        // §10.4.2: {w: true, e: false, c: false}. v0: `length` is always writable.
        let len = num(f64::from(object::array_len(o)));
        return Some(Desc::data(len, attr::WRITABLE));
    }
    own_property(cx, o, key).map(|(v, a, _)| Desc::data(v, a))
}

/// IsExtensible (§7.2.5, ordinary objects).
pub(crate) fn is_extensible(o: u64) -> bool {
    object::flags_of(o) & obj::EXTENSIBLE != 0
}

/// `o.[[PreventExtensions]]()` (§10.1.4.1). Always succeeds for ordinary objects.
pub(crate) fn prevent_extensions(rt: &Runtime, o: u64) -> bool {
    let flags = object::flags_of(o) & !obj::EXTENSIBLE;
    object::wr(&rt.heap, object::cell(o), obj::FLAGS, int32_bits(flags));
    true
}

/// `o.[[SetPrototypeOf]](p)` (§10.1.2.1 OrdinarySetPrototypeOf; `p` is an object or null).
/// %Object.prototype% is an immutable prototype exotic object (§10.4.7).
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
/// definition is rejected. v0: a descriptor with `get` or `set`, a non-writable array
/// `length`, and an array element with attributes other than {w, e, c} throw "not
/// implemented".
pub(crate) fn define_own_property(cx: &mut Cx, o: u64, key: u64, desc: Desc) -> Res<bool> {
    debug_assert!(is_object(o) && is_property_key(key));
    if desc.is_accessor() {
        return Err(not_implemented(cx, "accessor properties"));
    }
    if is_array(o) && is_atom(cx, key, Atom::Length) {
        return array_define_length(cx, o, desc);
    }
    let Some((cur_v, cur_a, slot)) = own_property(cx, o, key) else {
        // §10.1.6.3 step 2: a new property; absent fields default to undefined / false.
        if !is_extensible(o) {
            return Ok(false);
        }
        let v = desc.value.unwrap_or(UNDEFINED);
        return add_own_property(cx, o, key, v, desc.new_attrs());
    };
    if !validate_data(&desc, cur_v, cur_a) {
        return Ok(false);
    }
    let attrs = desc.merge_attrs(cur_a);
    let v = desc.value.unwrap_or(cur_v);
    if attrs == cur_a && v == cur_v {
        return Ok(true);
    }
    let rt = cx.rt();
    match slot {
        OwnSlot::Table => own_define(rt, o, key, v, attrs),
        OwnSlot::Element(i) => {
            if attrs != attr::DEFAULT {
                return Err(not_implemented(
                    cx,
                    "array elements that are not writable, enumerable and configurable",
                ));
            }
            object::array_set(rt, o, i, v);
        }
        OwnSlot::FunctionVirtual => {
            // The property moves into the table; [[OwnPropertyKeys]] keeps it first.
            let key = intern_key(rt, key);
            own_define(rt, o, key, v, attrs);
        }
        // Validation passed on a non-writable, non-configurable property: nothing changes
        // (SameValue held for the value).
        OwnSlot::WrapperVirtual => {}
    }
    Ok(true)
}

/// ValidateAndApplyPropertyDescriptor (§10.1.6.3) steps 4-5 for a data descriptor (or a
/// generic one) against an existing data property.
fn validate_data(desc: &Desc, cur_v: u64, cur_a: i32) -> bool {
    if cur_a & attr::CONFIGURABLE != 0 {
        return true;
    }
    if desc.configurable == Some(true) {
        return false;
    }
    if desc
        .enumerable
        .is_some_and(|e| e != (cur_a & attr::ENUMERABLE != 0))
    {
        return false;
    }
    if cur_a & attr::WRITABLE == 0 {
        if desc.writable == Some(true) {
            return false;
        }
        if desc.value.is_some_and(|v| !same_value(v, cur_v)) {
            return false;
        }
    }
    true
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
        // Past the v0 dense limit the index becomes a table property and `length` stays
        // (H18 v0 notes), as in [[Set]] and literals.
    }
    let key = intern_key(rt, key);
    own_define(rt, o, key, v, attrs);
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
/// then symbols in creation order. The virtual properties come where the spec creates them:
/// `length` of an array or String wrapper first among the strings; `length`, `name`, then
/// `prototype` of a function. GC point.
pub(crate) fn own_property_keys(cx: &mut Cx, o: u64) -> u64 {
    debug_assert!(is_object(o));
    let rt = cx.rt();
    // GC: `keys` is a Rust local (a root by the stack scan); the table keys stay reachable
    // from `o`, and no JS runs here, so the table does not change under the loops.
    let keys = object::new_array(rt, 0);
    let push = |k: u64| {
        let n = object::array_len(keys);
        object::array_set(rt, keys, n, k);
    };
    // 1. Integer indices ascending: elements, String wrapper indices, table index keys.
    let mut indices: Vec<u32> = Vec::new();
    if is_array(o) {
        indices.extend((0..object::array_len(o)).filter(|&i| object::array_get(o, i).is_some()));
    }
    let wrapped_string = wrapper::wrapper_value(o).filter(|&p| is_string(p));
    if let Some(s) = wrapped_string {
        indices.extend(0..object::str_len(s) as u32);
    }
    let table = |i: usize| object::own_entry(o, i).0;
    indices.extend((0..object::own_count(o)).filter_map(|i| array_index_of(table(i))));
    indices.sort_unstable();
    indices.dedup();
    for i in indices {
        let s = crate::conv::number_string(cx, f64::from(i));
        push(s);
    }
    // 2. Strings in creation order, the virtual ones first.
    let mut first: [Option<Atom>; 3] = [None; 3];
    if is_array(o) || wrapped_string.is_some() {
        first[0] = Some(Atom::Length);
    } else if is_function(o) {
        first = [Some(Atom::Length), Some(Atom::Name), None];
        if object::function_info(o).is_constructor() {
            first[2] = Some(Atom::Prototype);
        }
    }
    for a in first.into_iter().flatten() {
        push(rt.atom(a));
    }
    let is_first = |k: u64| {
        first
            .into_iter()
            .flatten()
            .any(|a| object::key_eq(k, rt.atom(a)))
    };
    for i in 0..object::own_count(o) {
        let k = table(i);
        if !is_symbol_key(k) && array_index_of(k).is_none() && !is_first(k) {
            push(k);
        }
    }
    // 3. Symbols in creation order.
    for i in 0..object::own_count(o) {
        let k = table(i);
        if is_symbol_key(k) {
            push(k);
        }
    }
    keys
}
