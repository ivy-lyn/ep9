//! Property access on v0 objects: [[Get]], [[Set]], [[DefineOwnProperty]], [[Delete]],
//! [[HasProperty]] along the prototype chain (§10.1). v0: data properties only (no accessors),
//! string keys only (no symbols), dense array elements only.
//!
//! | Receiver | Own properties seen before the table |
//! |---|---|
//! | array | `length`, array-index keys (elements) |
//! | function | `prototype` (created on first access for constructors), `length`, `name` |
//! | string primitive | `length`, array-index keys; then `String.prototype` |
//! | number / boolean primitive | `Number.prototype` / `Boolean.prototype` |

use crate::conv::{Res, describe, num, to_property_key, to_uint32_of};
use crate::error::throw_error;
use crate::frame::{NULL, UNDEFINED};
use crate::interp::Cx;
use crate::key::{array_index_of, is_property_key, key_text};
use crate::kinds::is_callable;
use crate::object::{self, attr, is_array, is_function, is_object, is_string, own_define, own_get};
use crate::runtime::{Atom, ErrorKind, Root, val};
use crate::string;

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
    let rt = cx.rt();
    if is_array(base) {
        if is_atom(cx, key, Atom::Length) {
            let n = to_uint32_of(cx, v)?;
            let d = crate::conv::to_number(cx, v)?;
            if f64::from(n) != d || !object::array_set_length(rt, base, n) {
                return Err(throw_error(
                    cx,
                    ErrorKind::RangeError,
                    "Invalid array length",
                ));
            }
            return Ok(());
        }
        if let Some(i) = array_index_of(key)
            && object::array_set(rt, base, i, v)
        {
            return Ok(());
        }
    }
    if is_function(base) && object::function_info(base).is_constructor() {
        // Materialize `prototype` so the store below replaces it.
        if is_atom(cx, key, Atom::Prototype) {
            function_prototype_property(cx, base);
        }
    }
    // Find the property on the chain to check writability.
    let mut o = base;
    loop {
        if let Some((_, a)) = own_get(o, key) {
            if a & attr::WRITABLE == 0 {
                return fail(cx, key, strict);
            }
            break;
        }
        let p = object::proto_of(o);
        if p == NULL {
            break;
        }
        o = p;
    }
    if object::own_replace(rt, base, key, v) {
        return Ok(());
    }
    if object::flags_of(base) & object::obj::EXTENSIBLE == 0 {
        return fail(cx, key, strict);
    }
    own_define(rt, base, key, v, attr::DEFAULT);
    Ok(())
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
        && object::array_set(cx.rt(), base, i as u32, v)
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
    let mut cur = o;
    loop {
        if virtual_get(cx, cur, key).is_some() || own_get(cur, key).is_some() {
            return Ok(true);
        }
        let p = object::proto_of(cur);
        if p == NULL {
            return Ok(false);
        }
        cur = p;
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
