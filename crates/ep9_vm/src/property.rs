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
//! | proxy (PROXY, PROXY_CALLABLE) | routed hooks (`exotic.rs`): `proxy.rs`, all internal methods |
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
//!
//! Proxies (M4-P4 PROXY1; plan v9 §6.3, V9-A21). PROXY and PROXY_CALLABLE are routed kinds
//! whose internal methods may run JS (the traps, PROXY2 in month 5) and may throw (a revoked
//! proxy). The ordinary functions keep their infallible signatures and assert in debug
//! builds that they never meet a proxy: [`has_property`], [`has_own_property`],
//! [`is_extensible`], [`prevent_extensions`], [`set_prototype_of`], [`own_property_keys`],
//! [`has_index`] and `exotic::own_attrs`. `[[GetPrototypeOf]]` is `object::proto_of` (no
//! assert: it is in the IC code; a proxy cell's own shape has a null prototype). Each has a
//! fallible form that is a GC point and asks a proxy: [`try_has_property`],
//! [`try_has_own_property`], [`try_is_extensible`], [`try_prevent_extensions`],
//! [`try_set_prototype_of`], [`try_own_property_keys`], [`try_has_index`],
//! `exotic::try_own_attrs` and [`try_get_prototype_of`]. Code that can meet a proxy (any JS
//! object) calls the fallible form; the callers that keep the ordinary form hold an ordinary
//! object by construction, or are on the PROXY2 list (month 5).
//!
//! ```text
//!  get / set / define / delete / [[GetOwnProperty]]: the routed hook (exotic.rs) -> proxy.rs
//!  has / in / has_index ............................: try_has_property, try_has_index
//!  a walk over [[Prototype]] that may meet a proxy  : try_get_prototype_of
//! ```

use crate::conv::{Res, describe, num, to_property_key, to_uint32_of};
use crate::descriptor::Desc;
use crate::error::{
    Part::Key, Part::Text, bounded_message, not_implemented, stack_overflow, throw_error,
};
use crate::exotic::{self, Exotic};
use crate::frame::{NULL, UNDEFINED};
use crate::interp::Cx;
use crate::key::{self, Probe, array_index_of, intern_key, is_property_key, is_symbol_key};
use crate::kinds::{self as kind, is_bound, is_callable, is_symbol};
use crate::object::{self, attr, is_array, is_function, is_object, is_string};
use crate::props;
use crate::runtime::{Atom, ErrorKind, Root, Runtime, val};
use crate::{accessor, arguments, bound, elements, shape, string, symbol, wrapper};

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
        // A plain object with an elements cell asks the hook too (its index keys, H18-Q3).
        if kind::cell_kind(o) != kind::OBJECT.get() || object::has_elements_cell(o) {
            if let Some(r) = exotic::get(cx, o, key, how, receiver) {
                return r;
            }
            // An entry of a DICTIONARY array (H18, M5-P2 ELEM2): exotic.rs reads only the
            // dense store.
            if let Some((v, a)) = dict_index(o, key) {
                if a & attr::ACCESSOR == 0 {
                    return Ok(v);
                }
                return accessor::call_getter(cx, v, receiver);
            }
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
        if (k != kind::OBJECT.get() || object::has_elements_cell(o)) && !exotic::is_routed(k) {
            // Only non-routed kinds get here, and their `exotic::get` never throws; the
            // `Some(Err)` arm is defensive.
            match exotic::get(cx, o, key, how, o) {
                Some(Ok(v)) => return Some(v),
                Some(Err(_)) => {
                    cx.rt().take_pending();
                    return None;
                }
                None => {}
            }
            if let Some((v, a)) = dict_index(o, key) {
                return (a & attr::ACCESSOR == 0).then_some(v);
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
        let msg = bounded_message(&[
            Text("Cannot read properties of "),
            Key(base),
            Text(" (reading '"),
            Key(key),
            Text("')"),
        ]);
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

/// The start of a string, number, boolean or symbol in V8's message form (`string 'ab'`,
/// `number '1.5'`, `boolean 'true'`, `symbol 'Symbol(x)'`): the type and the opening quote;
/// the value's text is a [`Key`] part (bounded, H19-Q5).
fn primitive_type(b: u64) -> &'static str {
    if is_string(b) {
        "string '"
    } else if is_symbol(b) {
        "symbol '"
    } else if val(b).is_number() {
        "number '"
    } else {
        "boolean '"
    }
}

/// `base[k]` for any key value (keyed load). An int32 index `>= 0` on an object makes no
/// string key (M4-P3 KEYED, OI-04): an array element, an arguments element ([`get_int`]),
/// then [`get_index`] (a hole, a String wrapper's character, a stored index, the chain).
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
        if is_object(base) {
            return get_int(cx, base, i as u32);
        }
    }
    if base == UNDEFINED || base == NULL {
        // The key's length is checked before its text is read (H19-Q5, Codex H19 H-1): a
        // rope key near the length limit gives `<error>` with no flatten and no copy.
        let msg = bounded_message(&[
            Text("Cannot read properties of "),
            Key(base),
            Text(" (reading '"),
            Key(k),
            Text("')"),
        ]);
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    let key = to_property_key(cx, k)?;
    get(cx, base, key)
}

/// `o[i]` (`[[Get]]` with receiver `o`) for the object `o` and an array index `i`, without a
/// string key: an element of an arguments object (a mapped index reads its parameter), then
/// [`get_index`], which answers an array hole, a String wrapper's character and a missing
/// index without a key, and asks a routed kind with the key. GC point (a getter, the key
/// path).
#[inline]
pub(crate) fn get_int(cx: &mut Cx, o: u64, i: u32) -> Res<u64> {
    if kind::cell_kind(o) == kind::ARGUMENTS.get()
        && let Some(v) = arguments::element_value(o, i)
    {
        return Ok(v);
    }
    get_index(cx, o, u64::from(i))
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
        let msg = bounded_message(&[
            Text("Cannot set properties of "),
            Key(base),
            Text(" (setting '"),
            Key(key),
            Text("')"),
        ]);
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
                // §10.1.9.2: a non-writable `length` fails before any conversion; else
                // ArraySetLength (§10.4.2.4) through `[[DefineOwnProperty]]`.
                if !elements::length_writable(o) {
                    return Ok(false);
                }
                let desc = Desc {
                    value: Some(v),
                    ..Desc::default()
                };
                return array_define_length(cx, o, desc);
            }
            if let Some(i) = array_index_of(key)
                && array_store(rt, o, i, v)
            {
                return Ok(true);
            }
        } else if elements::plain_elements_kind(kind::cell_kind(o))
            && object::has_elements_cell(o)
            && let Some(i) = array_index_of(key)
            && plain_store(rt, o, i, v)
        {
            return Ok(true);
        } else if is_function(o) && exotic::is_lazy_prototype(rt, o, key, how) {
            // Make `prototype`, so the store below replaces it.
            exotic::function_prototype(rt, o);
        }
    }
    // The first property on the chain decides (§10.1.9.2 steps 1-2 and 4-7).
    let mut cur = o;
    loop {
        let found = match own_attrs(rt, cur, key, how) {
            // An entry of a DICTIONARY array may be an accessor: its pair (M5-P2 ELEM2).
            Some(a) if is_dict_array(cur) => dict_own(rt, cur, key, how).or(Some((UNDEFINED, a))),
            // Other exotic own properties are data properties: the value is not needed.
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
/// `own_property` does not see) is an existing data property: step 2.c.iii returns false
/// for a non-writable one before any conversion; else step 2.d.iv goes to ArraySetLength (a
/// RangeError for an invalid length). GC point.
fn define_on_receiver(cx: &mut Cx, receiver: u64, key: u64, v: u64) -> Res<bool> {
    if !is_object(receiver) {
        return Ok(false);
    }
    // Steps 2.d.iii-iv: only the value; the attributes stay.
    let desc = Desc {
        value: Some(v),
        ..Desc::default()
    };
    if is_array(receiver) && is_atom(cx, key, Atom::Length) {
        // Step 2.c.iii (Codex M5-P2 r1 elem2 2): no conversion of `v`.
        if !elements::length_writable(receiver) {
            return Ok(false);
        }
        return define_own_property(cx, receiver, key, desc);
    }
    match own_property(cx, receiver, key)? {
        Some((_, a, _)) if a & attr::WRITABLE == 0 => Ok(false),
        Some(_) => define_own_property(cx, receiver, key, desc),
        // Step 2.e: CreateDataProperty(Receiver, P, V). A proxy receiver takes its
        // `[[DefineOwnProperty]]` (its cell's storage holds no property key).
        None if crate::proxy::is_proxy_kind(kind::cell_kind(receiver)) => {
            define_own_property(cx, receiver, key, Desc::data(v, attr::DEFAULT))
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

/// [`array_store`] for an object `o` of an `elements::plain_elements_kind` (H18-Q3): an
/// existing element takes the value; a missing one is added to the elements store only if
/// `o` has a fast shape (extensible, and no index key among its stored properties: an index
/// key makes a dictionary, OM-10) and no prototype has an index key. False if it stored
/// nothing (also an index too far for the elements store, `object::element_set`): the caller
/// takes the key path. GC point.
fn plain_store(rt: &Runtime, o: u64, i: u32, v: u64) -> bool {
    if object::element_get(o, i).is_none()
        && (shape::is_dictionary(shape::shape_of(o)) || shape::proto_index_keys(rt))
    {
        return false;
    }
    object::element_set(rt, o, i, v)
}

/// Adds the new element `i` with the value `v` to the plain-elements object `o` for
/// CreateDataProperty (no chain walk): only if `o` has a fast shape (extensible, no stored
/// index key). False if it stored nothing: the caller takes the key path. GC point.
fn plain_define(rt: &Runtime, o: u64, i: u32, v: u64) -> bool {
    if object::element_get(o, i).is_none() && shape::is_dictionary(shape::shape_of(o)) {
        return false;
    }
    object::element_set(rt, o, i, v)
}

// ---- DICTIONARY elements of Arrays (M5-P2 ELEM2; H18-Q2, Q4) ---------------------------------
//
// An Array of kind DICTIONARY keeps its elements in an ELEMENTS_DICT cell
// (`elements::dict`): index -> (value or ACCESSOR_PAIR, attributes), and the flag of a
// non-writable `length`. `exotic.rs` reads only the dense store (the dict cell's dense
// capacity is 0, so its Array hooks find no element), so the paths below ask the cell
// first: [`dict_index`] for `[[Get]]`, [`own_attrs`] and [`dict_own`] for `[[Set]]`,
// HasOwnProperty and CreateDataProperty, `own_property` for `[[GetOwnProperty]]` and
// `[[DefineOwnProperty]]`, [`dict_delete`] for `[[Delete]]`, `own_index` for the
// integer-index paths and the key lists. An Array keeps no index key in its property
// storage.

/// True if `o` is an Array of elements kind DICTIONARY. Not a GC point.
#[inline(always)]
fn is_dict_array(o: u64) -> bool {
    kind::cell_kind(o) == kind::ARRAY.get() && elements::kind_of(o) == elements::kind::DICTIONARY
}

/// Entry `key` (an array index) of the DICTIONARY Array `o`: (value or ACCESSOR_PAIR,
/// attributes). `None` for another object or key. Not a GC point.
#[inline]
fn dict_index(o: u64, key: u64) -> Option<(u64, i32)> {
    if !is_dict_array(o) {
        return None;
    }
    let d = elements::dict_of(o)?;
    elements::dict_get(d, array_index_of(key)?)
}

/// The own property `key` of the DICTIONARY Array `o` that `exotic.rs` does not answer
/// right: an entry of its cell, or `length` when it is not writable (attributes 0). Not a GC
/// point.
fn dict_own(rt: &Runtime, o: u64, key: u64, how: Probe) -> Option<(u64, i32)> {
    let d = elements::dict_of(o)?;
    if let Some(i) = array_index_of(key) {
        return elements::dict_get(d, i);
    }
    (!elements::length_writable(o) && exotic::is_key(rt, key, how, Atom::Length))
        .then(|| (num(f64::from(object::array_len(o))), 0))
}

/// `exotic::own_attrs` with the DICTIONARY entries and a non-writable `length` of an Array.
/// Not a GC point.
#[inline(always)]
fn own_attrs(rt: &Runtime, o: u64, key: u64, how: Probe) -> Option<i32> {
    if is_dict_array(o)
        && let Some((_, a)) = dict_own(rt, o, key, how)
    {
        return Some(a);
    }
    exotic::own_attrs(rt, o, key, how)
}

/// The attributes of the Array `a`'s `length`: {w} or none (DICTIONARY with the flag).
fn length_attrs(a: u64) -> i32 {
    if elements::length_writable(a) {
        attr::WRITABLE
    } else {
        0
    }
}

/// Element `i` of the Array `a` becomes `v` (a Value or an ACCESSOR_PAIR) with `attrs`: a
/// DICTIONARY entry (the array moves there first, one-way). An index at or past `length`
/// raises it (§10.4.2.1 step 3; the callers checked that `length` is writable). A new entry
/// on a prototype sets `shape::proto_index_keys`; any change of a prototype's element is a
/// structural change (OM-8). GC point: `a` and `v` live in Rust locals.
fn dict_define(rt: &Runtime, a: u64, i: u32, v: u64, attrs: i32) {
    debug_assert!(is_array(a) && i != u32::MAX);
    elements::to_dictionary(rt, a);
    let present = elements::dict_of(a)
        .and_then(|d| elements::dict_get(d, i))
        .is_some();
    elements::dict_put(rt, a, i, v, attrs);
    if i >= object::array_len(a) {
        object::set_array_len(rt, a, i + 1);
    }
    if shape::is_prototype_object(a) {
        if !present {
            shape::note_proto_index_key(rt, a);
        }
        shape::note_change(rt, a);
    }
}

/// `[[Delete]]` of entry `i` of the DICTIONARY Array `a`: false for a non-configurable entry,
/// else removed (true, also for none). Not a GC point.
fn dict_delete(rt: &Runtime, a: u64, i: u32) -> bool {
    let Some(d) = elements::dict_of(a) else {
        return true;
    };
    match elements::dict_get(d, i) {
        Some((_, at)) if at & attr::CONFIGURABLE == 0 => false,
        Some(_) => {
            elements::dict_remove(d, i);
            shape::note_change(rt, a);
            true
        }
        None => true,
    }
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
    // `what`: the receiver as V8 names it, `#<Object>` or a primitive's `type 'text'`.
    let what = if is_object(base) {
        [Text("#<Object>"), Text(""), Text("")]
    } else {
        [Text(primitive_type(base)), Key(base), Text("'")]
    };
    let own_string = is_string(base)
        && (is_atom(cx, key, Atom::Length)
            || array_index_of(key).is_some_and(|i| (i as usize) < object::str_len(base)));
    let found = if own_string {
        Some(0)
    } else {
        first_attrs_on_chain(rt, start, key)
    };
    let [w0, w1, w2] = what;
    let k = Key(key);
    let msg = match found {
        Some(a) if a & attr::ACCESSOR != 0 => bounded_message(&[
            Text("Cannot set property "),
            k,
            Text(" of "),
            w0,
            w1,
            w2,
            Text(" which has only a getter"),
        ]),
        Some(a) if a & attr::WRITABLE == 0 && is_object(base) => bounded_message(&[
            Text("Cannot assign to read only property '"),
            k,
            Text("' of object '"),
            w0,
            w1,
            w2,
            Text("'"),
        ]),
        Some(a) if a & attr::WRITABLE == 0 => bounded_message(&[
            Text("Cannot assign to read only property '"),
            k,
            Text("' of "),
            w0,
            w1,
            w2,
        ]),
        _ if is_object(base) => bounded_message(&[
            Text("Cannot add property "),
            k,
            Text(", object is not extensible"),
        ]),
        _ => bounded_message(&[
            Text("Cannot create property '"),
            k,
            Text("' on "),
            w0,
            w1,
            w2,
        ]),
    };
    Err(throw_error(cx, ErrorKind::TypeError, &msg))
}

/// The attributes of the first property `key` on the chain from the object `o`, if any, for
/// an error message. Not a GC point and runs no JS (a lazy `prototype` is not made: it is
/// {w}). A proxy is not asked: the walk goes on at its target, as the no-trap step does (a
/// revoked proxy ends it). A proxy may make the chain a cycle: past [`MAX_PROXY_HOPS`]
/// proxies the walk gives `None` (M4-P4 fixer B, Codex PROXY1 P-2).
fn first_attrs_on_chain(rt: &Runtime, mut o: u64, key: u64) -> Option<i32> {
    let how = key::probe(key);
    let mut hops = 0;
    loop {
        while crate::proxy::is_proxy_kind(kind::cell_kind(o)) {
            hops += 1;
            if hops > MAX_PROXY_HOPS {
                return None;
            }
            o = crate::proxy::target(o);
            if o == NULL {
                return None;
            }
        }
        if let Some(a) =
            own_attrs(rt, o, key, how).or_else(|| props::own_get_with(o, key, how).map(|(_, a)| a))
        {
            return Some(a);
        }
        o = object::proto_of(o);
        if o == NULL {
            return None;
        }
    }
}

/// `base[k] = v` for any key value (keyed store). An int32 index `>= 0` stores a dense array
/// element ([`array_store`]) or a writable arguments element (its parameter too while it is
/// mapped, §10.4.4.4) without a string key (M4-P3 KEYED); every other case takes the key.
pub(crate) fn set_keyed(cx: &mut Cx, base: u64, k: u64, v: u64, strict: bool) -> Res<()> {
    if let Some(i) = val(k).as_int32()
        && i >= 0
    {
        let kd = kind::cell_kind(base);
        if kd == kind::ARRAY.get() && array_store(cx.rt(), base, i as u32, v) {
            return Ok(());
        }
        if elements::plain_elements_kind(kd) && plain_store(cx.rt(), base, i as u32, v) {
            return Ok(());
        }
        if kd == kind::ARGUMENTS.get() && arguments::set_element_value(cx.rt(), base, i as u32, v) {
            return Ok(());
        }
    }
    if base == UNDEFINED || base == NULL {
        let msg = bounded_message(&[
            Text("Cannot set properties of "),
            Key(base),
            Text(" (setting '"),
            Key(k),
            Text("')"),
        ]);
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
    {
        // An existing dense element is {w, e, c}; a new one needs an extensible array. A
        // DICTIONARY array's `array_set` refuses an entry with other attributes and a new
        // index past a non-writable `length`: `[[DefineOwnProperty]]` decides those.
        if (object::array_get(o, i).is_some() || is_extensible(o)) && object::array_set(rt, o, i, v)
        {
            return Ok(());
        }
        let exists = own_attrs(rt, o, key, how).is_some();
        return define_slow(cx, o, key, v, exists);
    }
    // An index key of a plain-elements object that has an elements cell (H18-Q3): its
    // element, or a new one when the property storage has no such key (one place per key).
    // Without a cell, a string index key takes the key path (OM-10), as in v0: the index test
    // is a call, and every named define would pay it (M5-P1 BENCH-SET, ci-v0).
    if elements::plain_elements_kind(kind::cell_kind(o))
        && object::has_elements_cell(o)
        && let Some(i) = array_index_of(key)
        && (object::element_get(o, i).is_some()
            || (is_extensible(o) && props::own_get_with(o, key, how).is_none()))
        && object::element_set(rt, o, i, v)
    {
        return Ok(());
    }
    // A routed kind (ARGUMENTS, MODULE_NAMESPACE, a proxy) takes its `[[DefineOwnProperty]]`
    // hook.
    if exotic::is_routed(kind::cell_kind(o)) {
        return define_routed(cx, o, key, v);
    }
    let exotic = own_attrs(rt, o, key, how);
    let stored = props::own_get_with(o, key, how);
    if exotic.is_none() {
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
    define_slow(cx, o, key, v, exists)
}

/// [`define`] on a routed kind (out of line: the fast path above stays as it was). The
/// message of a rejection says whether the property existed; for a proxy that would need its
/// `[[GetOwnProperty]]` (a trap from PROXY2 on), so a proxy gets V8's trap text. GC point.
#[cold]
#[inline(never)]
fn define_routed(cx: &mut Cx, o: u64, key: u64, v: u64) -> Res<()> {
    if crate::proxy::is_proxy_kind(kind::cell_kind(o)) {
        if define_own_property(cx, o, key, Desc::data(v, attr::DEFAULT))? {
            return Ok(());
        }
        let msg = bounded_message(&[
            Text("'defineProperty' on proxy: trap returned falsish for property '"),
            Key(key),
            Text("'"),
        ]);
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    let how = key::probe(key);
    let exists =
        own_attrs(cx.rt(), o, key, how).is_some() || props::own_get_with(o, key, how).is_some();
    define_slow(cx, o, key, v, exists)
}

/// The `[[DefineOwnProperty]]` step of [`define`]; a rejection throws a TypeError that says
/// whether the property `exists`. GC point.
fn define_slow(cx: &mut Cx, o: u64, key: u64, v: u64, exists: bool) -> Res<()> {
    if define_own_property(cx, o, key, Desc::data(v, attr::DEFAULT))? {
        return Ok(());
    }
    let msg = if exists {
        bounded_message(&[Text("Cannot redefine property: "), Key(key)])
    } else {
        bounded_message(&[
            Text("Cannot define property "),
            Key(key),
            Text(", object is not extensible"),
        ])
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
    if elements::plain_elements_kind(kind::cell_kind(o))
        && let Some(i) = val(k).as_int32()
        && i >= 0
        && plain_define(cx.rt(), o, i as u32, v)
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
    if internal_delete(cx, base, key)? {
        Ok(true)
    } else {
        deny(cx, key, strict)
    }
}

/// `o.[[Delete]](key)` (§10.1.10, or a routed kind's hook) on the object `o`. GC point.
pub(crate) fn internal_delete(cx: &mut Cx, o: u64, key: u64) -> Res<bool> {
    match exotic::delete(cx, o, key) {
        Some(r) => r,
        None => Ok(ordinary_delete(cx.rt(), o, key)),
    }
}

/// OrdinaryDelete (§10.1.10.1) of the property key `key` on the object `o`, past the
/// `[[Delete]]` hook of `o`: false for a non-configurable own property. GC point (a
/// dictionary move).
pub(crate) fn ordinary_delete(rt: &Runtime, o: u64, key: u64) -> bool {
    let how = key::probe(key);
    if is_dict_array(o)
        && let Some(i) = array_index_of(key)
    {
        // An entry of a DICTIONARY array (M5-P2 ELEM2): a non-configurable one stays.
        return dict_delete(rt, o, i);
    }
    if is_array(o)
        && let Some(i) = array_index_of(key)
        && object::array_get(o, i).is_some()
    {
        // v0: a dense array keeps a hole.
        object::array_set(rt, o, i, crate::frame::HOLE);
        return true;
    }
    if elements::plain_elements_kind(kind::cell_kind(o))
        && let Some(i) = array_index_of(key)
        && object::element_get(o, i).is_some()
    {
        // An element is {w, e, c}: it becomes a hole.
        object::element_set(rt, o, i, crate::frame::HOLE);
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
        let msg = bounded_message(&[
            Text("Cannot delete property '"),
            Key(key),
            Text("' of object"),
        ]);
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
    // An int32 index `>= 0` needs no string key (M4-P3 KEYED): [`try_has_index`] answers an
    // element, a String wrapper index and a missing index, and asks a routed kind with the
    // key (an arguments element answers first).
    if let Some(i) = val(k).as_int32()
        && i >= 0
    {
        if kind::cell_kind(o) == kind::ARGUMENTS.get()
            && arguments::element_value(o, i as u32).is_some()
        {
            return Ok(true);
        }
        return try_has_index(cx, o, i as u64);
    }
    let key = to_property_key(cx, k)?;
    try_has_property(cx, o, key)
}

/// HasProperty(O, P) (§7.3.12, OrdinaryHasProperty along the chain) for a property key, for
/// a chain with no proxy (a debug assert at each object; [`try_has_property`] takes one).
/// Runs no JS and allocates nothing.
// V9-A21 keeps the ordinary form; since `Cx::has_index` asks [`try_has_index`] (M4-P4 fixer
// B, Codex PROXY1 P-3) only the tests call it.
#[cfg_attr(not(test), allow(dead_code))]
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

/// HasProperty(O, P) (§7.3.12) for any object: OrdinaryHasProperty along the chain until a
/// proxy, whose `[[HasProperty]]` (`proxy.rs`) answers for itself and the rest of the
/// chain. GC point (a proxy may run JS from PROXY2 on); a chain with no proxy allocates
/// nothing.
pub(crate) fn try_has_property(cx: &mut Cx, o: u64, key: u64) -> Res<bool> {
    debug_assert!(is_object(o) && is_property_key(key));
    let how = key::probe(key);
    let mut cur = o;
    loop {
        if crate::proxy::is_proxy_kind(kind::cell_kind(cur)) {
            // GC point: `key` lives in a Rust local.
            return crate::proxy::has_property(cx, cur, key);
        }
        if has_own_with(cx.rt(), cur, key, how) {
            return Ok(true);
        }
        let p = object::proto_of(cur);
        if p == NULL {
            return Ok(false);
        }
        cur = p;
    }
}

/// HasOwnProperty(O, P) (§7.3.13): `[[GetOwnProperty]]` is not undefined, for an object that
/// is not a proxy (a debug assert; [`try_has_own_property`] takes one). Allocates nothing.
pub(crate) fn has_own_property(cx: &Cx, o: u64, key: u64) -> bool {
    debug_assert!(is_object(o) && is_property_key(key));
    has_own_with(cx.rt(), o, key, key::probe(key))
}

/// HasOwnProperty(O, P) (§7.3.13) for any object. ARGUMENTS and MODULE_NAMESPACE ask their
/// `[[GetOwnProperty]]` hook (a namespace binding in its TDZ throws, §10.4.6.5); a proxy its
/// `[[GetOwnProperty]]` (`exotic::try_own_attrs`). GC point for a routed kind only.
pub(crate) fn try_has_own_property(cx: &mut Cx, o: u64, key: u64) -> Res<bool> {
    debug_assert!(is_object(o) && is_property_key(key));
    let k = kind::cell_kind(o);
    if crate::proxy::is_proxy_kind(k) {
        return Ok(exotic::try_own_attrs(cx, o, key, key::probe(key))?.is_some());
    }
    if exotic::is_routed(k) {
        return Ok(get_own_property(cx, o, key)?.is_some());
    }
    Ok(has_own_property(cx, o, key))
}

fn has_own_with(rt: &Runtime, o: u64, key: u64, how: Probe) -> bool {
    debug_assert!(
        !crate::proxy::is_proxy_kind(kind::cell_kind(o)),
        "the ordinary HasProperty met a proxy: use try_has_property"
    );
    props::own_get_with(o, key, how).is_some() || own_attrs(rt, o, key, how).is_some()
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
    // The InstanceOf IC (M4-P3 IC2, `instance_ic.rs`): the site, read before any JS runs;
    // the case of `f` is recorded once the generic answer is known.
    let site = crate::instance_ic::site(cx);
    let r = instance_of_generic(cx, v, f);
    if r.is_ok()
        && let Some((vector, slot)) = site
    {
        // GC point: `f` lives in a Rust local.
        crate::instance_ic::record(cx.rt(), vector, slot, f);
    }
    r
}

/// InstanceofOperator(v, f) without the IC: the loop of [`instance_of`].
fn instance_of_generic(cx: &mut Cx, v: u64, f: u64) -> Res<bool> {
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
    // 6. Repeat: O = ? O.[[GetPrototypeOf]](); null gives false, P gives true. GC point (a
    //    proxy on the chain, the interrupt poll): `proto` lives in a Rust local. A proxy may
    //    make the chain a cycle, so the walk is bounded ([`prototype_walk_step`]).
    let mut hops = 0;
    let mut o = prototype_walk_step(cx, o, &mut hops)?;
    while o != NULL {
        if o == proto {
            return Ok(true);
        }
        o = prototype_walk_step(cx, o, &mut hops)?;
    }
    Ok(false)
}

/// The most proxies one prototype-chain walk passes: V8's `JSProxy::kMaxIterationLimit`
/// (100 × 1024), past which V8's `PrototypeIterator` throws the RangeError "Maximum call
/// stack size exceeded" (node v24.22.1 gives it for `a instanceof C`, `isPrototypeOf` and
/// `__lookupGetter__` when `a`'s prototype is a proxy of `a`). Only a proxy can close a
/// cycle: OrdinarySetPrototypeOf refuses one through ordinary objects and stops its check at
/// a proxy (§10.1.2.1 step 8.c.i), so an ordinary chain of any length is never cut.
pub(crate) const MAX_PROXY_HOPS: u32 = 100 * 1024;

/// One step of a walk up the prototype chain (instanceof, for-in, and through
/// `Cx::prototype_walk_step` the builtins `isPrototypeOf`, `__lookupGetter__` and
/// `__lookupSetter__`; M4-P4 fixer B, Codex PROXY1 P-2, R-VM-P4-2): one interrupt budget unit
/// and a poll when it runs out (the walk has no back edge, M2-P2-POLL), then
/// `o.[[GetPrototypeOf]]()`. `hops` counts the proxies the walk has asked; past
/// [`MAX_PROXY_HOPS`] the step throws the RangeError "Maximum call stack size exceeded"
/// instead. The caller starts `hops` at 0. GC point (a proxy, the poll).
///
/// The hot part is inlined: the budget unit and, for an ordinary object, its shape's
/// prototype; the proxy part is a cold call (M4-P4 verifier B V-B1: the walk had cost about
/// 22 more instructions per step than before fixer B). The instanceof cache's hit walk takes
/// 8 units per full group of 8 steps (none for its last 1 to 8) and leaves the poll to this walk.
#[inline]
pub(crate) fn prototype_walk_step(cx: &mut Cx, o: u64, hops: &mut u32) -> Res<u64> {
    cx.budget -= 1;
    if cx.budget <= 0 && crate::interp::slow::interrupt(cx, core::ptr::null_mut()) == 0 {
        return Err(ep9_value::Throw);
    }
    if !crate::proxy::is_proxy_kind(kind::cell_kind(o)) {
        return Ok(object::proto_of(o));
    }
    proxy_walk_step(cx, o, hops)
}

/// The proxy part of [`prototype_walk_step`]: counts the hop, then the proxy's
/// `[[GetPrototypeOf]]` (`proxy::get_prototype_of`). GC point.
#[cold]
#[inline(never)]
fn proxy_walk_step(cx: &mut Cx, o: u64, hops: &mut u32) -> Res<u64> {
    *hops += 1;
    if *hops > MAX_PROXY_HOPS {
        return stack_overflow(cx);
    }
    crate::proxy::get_prototype_of(cx, o)
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
    // local, so a root) at once. A routed kind's keys come from its key array (`own`), and
    // each such array goes into `kept` (a Rust local, so a root) until the loop ends: the
    // keys in `seen_string` stay alive although a proxy's keys may be reachable from nothing
    // else once its traps run JS (PROXY2).
    let mut seen_index: Vec<u32> = Vec::new();
    let mut seen_string: Vec<u64> = Vec::new();
    let mut kept = UNDEFINED;
    let mut hops = 0;
    let mut cur = o;
    loop {
        // (index, enumerable) and (key, enumerable).
        let mut index_keys: Vec<(u32, bool)> = Vec::new();
        let mut string_keys: Vec<(u64, bool)> = Vec::new();
        // The key array of a routed kind, kept live until its keys are pushed below.
        let mut own = UNDEFINED;
        if exotic::is_routed(kind::cell_kind(cur)) {
            // A routed kind (ARGUMENTS, MODULE_NAMESPACE, a proxy): its `[[OwnPropertyKeys]]`
            // and `[[GetOwnProperty]]` (the hooks, else the ordinary methods). GC points.
            own = try_own_property_keys(cx, cur)?;
            if kept == UNDEFINED {
                kept = object::new_array(cx.rt(), 0);
            }
            let n = object::array_len(kept);
            object::array_set(cx.rt(), kept, n, own);
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
            if let Some(d) = elements::dict_of(cur) {
                // A DICTIONARY array's entries keep their own enumerable bits (M5-P2 ELEM2).
                index_keys.extend(
                    elements::dict_entries(d)
                        .into_iter()
                        .map(|(i, a)| (i, a & attr::ENUMERABLE != 0)),
                );
            } else if is_array(cur) || elements::plain_elements_kind(kind::cell_kind(cur)) {
                index_keys.extend(object::element_indices(cur).map(|i| (i, true)));
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
        // GC point (a proxy's [[GetPrototypeOf]], the interrupt poll). A proxy may make the
        // chain a cycle: the step is bounded ([`prototype_walk_step`]), so `kept` holds at
        // most one key array per proxy hop up to the bound.
        let p = prototype_walk_step(cx, cur, &mut hops)?;
        if p == NULL {
            break;
        }
        cur = p;
    }
    std::hint::black_box(kept);
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
    if is_dict_array(o)
        && let Some(i) = array_index_of(key)
    {
        // An entry of a DICTIONARY array (M5-P2 ELEM2): its value or ACCESSOR_PAIR and its
        // attributes; an Array keeps no index key in its property storage.
        return Ok(elements::dict_of(o)
            .and_then(|d| elements::dict_get(d, i))
            .map(|(v, a)| (v, a, OwnSlot::Exotic(Exotic::Element(i)))));
    }
    if let Some(r) = exotic::own_property(cx, o, key, how) {
        let (v, a, e) = r?;
        return Ok(Some((v, a, OwnSlot::Exotic(e))));
    }
    Ok(props::own_get_with(o, key, how).map(|(v, a)| (v, a, OwnSlot::Storage)))
}

/// `o.[[GetOwnProperty]](key)` in the stored form: (value, attribute bits), where an
/// accessor's value is its ACCESSOR_PAIR; an array's `length` too. The form a proxy's
/// `[[GetOwnProperty]]` returns from its target (`proxy.rs`). GC point.
pub(crate) fn own_property_stored(cx: &mut Cx, o: u64, key: u64) -> Res<Option<(u64, i32)>> {
    debug_assert!(is_object(o) && is_property_key(key));
    if is_array(o) && is_atom(cx, key, Atom::Length) {
        // §10.4.2: {w, e: false, c: false} (as `get_own_property`).
        return Ok(Some((
            num(f64::from(object::array_len(o))),
            length_attrs(o),
        )));
    }
    Ok(own_property(cx, o, key)?.map(|(v, a, _)| (v, a)))
}

/// `o.[[GetOwnProperty]](key)` (§10.1.5.1 OrdinaryGetOwnProperty, plus the exotic
/// properties of arrays, constructors and String wrappers; H20 R6). Only a routed kind's hook
/// throws (a namespace binding in its TDZ, §10.4.6.5). GC point.
pub(crate) fn get_own_property(cx: &mut Cx, o: u64, key: u64) -> Res<Option<Desc>> {
    debug_assert!(is_object(o) && is_property_key(key));
    if is_array(o) && is_atom(cx, key, Atom::Length) {
        // §10.4.2: {w, e: false, c: false}; not writable only on a DICTIONARY array (M5-P2).
        let len = num(f64::from(object::array_len(o)));
        return Ok(Some(Desc::data(len, length_attrs(o))));
    }
    Ok(own_property(cx, o, key)?.map(|(v, a, _)| accessor::stored_desc(v, a)))
}

/// IsExtensible (§7.2.5, ordinary objects): the shape's flag. Not for a proxy (a debug
/// assert; [`try_is_extensible`] takes one).
pub(crate) fn is_extensible(o: u64) -> bool {
    debug_assert!(
        !crate::proxy::is_proxy(o),
        "is_extensible on a proxy: use try_is_extensible"
    );
    shape::is_extensible(shape::shape_of(o))
}

/// IsExtensible(O) (§7.2.5) for any object: a proxy asks its `[[IsExtensible]]`. GC point
/// for a proxy only.
pub(crate) fn try_is_extensible(cx: &mut Cx, o: u64) -> Res<bool> {
    if crate::proxy::is_proxy(o) {
        return crate::proxy::is_extensible(cx, o);
    }
    Ok(is_extensible(o))
}

/// `o.[[PreventExtensions]]()` (§10.1.4.1). Always succeeds for ordinary objects. The
/// object moves to a non-extensible dictionary shape (OM-10). Not for a proxy (a debug
/// assert; [`try_prevent_extensions`] takes one). GC point.
pub(crate) fn prevent_extensions(rt: &Runtime, o: u64) -> bool {
    if is_extensible(o) {
        props::prevent_extensions(rt, o);
    }
    true
}

/// `o.[[PreventExtensions]]()` for any object: a proxy asks its own (`proxy.rs`). GC point.
pub(crate) fn try_prevent_extensions(cx: &mut Cx, o: u64) -> Res<bool> {
    if crate::proxy::is_proxy(o) {
        return crate::proxy::prevent_extensions(cx, o);
    }
    Ok(prevent_extensions(cx.rt(), o))
}

/// `o.[[GetPrototypeOf]]()` for any object: the shape's prototype (§10.1.1.1
/// OrdinaryGetPrototypeOf, `object::proto_of`), or a proxy's `[[GetPrototypeOf]]`
/// (`proxy.rs`). GC point for a proxy only.
#[inline]
pub(crate) fn try_get_prototype_of(cx: &mut Cx, o: u64) -> Res<u64> {
    if crate::proxy::is_proxy_kind(kind::cell_kind(o)) {
        return crate::proxy::get_prototype_of(cx, o);
    }
    Ok(object::proto_of(o))
}

/// `o.[[SetPrototypeOf]](p)` for any object (`p` is an object or null): a proxy asks its
/// own (`proxy.rs`). GC point.
pub(crate) fn try_set_prototype_of(cx: &mut Cx, o: u64, p: u64) -> Res<bool> {
    if crate::proxy::is_proxy(o) {
        return crate::proxy::set_prototype_of(cx, o, p);
    }
    Ok(set_prototype_of(cx.rt(), o, p))
}

/// `o.[[SetPrototypeOf]](p)` (§10.1.2.1 OrdinarySetPrototypeOf; `p` is an object or null).
/// %Object.prototype% is an immutable prototype exotic object (§10.4.7). Not for a proxy (a
/// debug assert; [`try_set_prototype_of`] takes one). GC point.
pub(crate) fn set_prototype_of(rt: &Runtime, o: u64, p: u64) -> bool {
    debug_assert!(is_object(o) && (p == NULL || is_object(p)));
    debug_assert!(
        !crate::proxy::is_proxy(o),
        "set_prototype_of on a proxy: use try_set_prototype_of"
    );
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
    // §10.4.2.1 step 3.b: an index at or past a non-writable `length` (DICTIONARY only).
    if is_array(o)
        && let Some(i) = array_index_of(key)
        && i >= object::array_len(o)
        && !elements::length_writable(o)
    {
        return Ok(false);
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
        if let OwnSlot::Exotic(e) = slot
            && !matches!(e, Exotic::Element(_))
        {
            // Exotic own properties are data properties; a String wrapper's are not
            // configurable, so validation refused them above.
            return Err(not_implemented(cx, "an accessor on an exotic property"));
        }
        // An element becomes an accessor: an Array's goes to its DICTIONARY store, a
        // plain-elements object's to the property storage (H18-Q3), below.
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
        // An Array element: a {w, e, c} data value of a dense array stays in place; anything
        // else is a DICTIONARY entry (H18-Q2; M5-P2 ELEM2).
        OwnSlot::Exotic(Exotic::Element(i)) if is_array(o) => {
            if attrs == attr::DEFAULT && !is_dict_array(o) {
                object::array_set(rt, o, i, v);
            } else {
                // GC point: `o` and `v` live in Rust locals.
                dict_define(rt, o, i, v, attrs);
            }
        }
        // An element of a plain-elements object (H18-Q3): a {w, e, c} data value stays an
        // element; anything else (an accessor, other attributes) moves it to the property
        // storage, as a v0 index key (one place per key: the element goes first).
        OwnSlot::Exotic(Exotic::Element(i)) => {
            if attrs == attr::DEFAULT {
                object::element_set(rt, o, i, v);
            } else {
                object::element_set(rt, o, i, crate::frame::HOLE);
                // GC point: `o`, `key` and `v` live in Rust locals.
                props::own_define(rt, o, key, v, attrs);
            }
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
    let rt = cx.rt();
    // GC point: `o`, `key` and the pair live in Rust locals.
    let pair = accessor::new_pair(rt, get, set);
    let attrs = attr::ACCESSOR | (attrs & (attr::ENUMERABLE | attr::CONFIGURABLE));
    if is_array(o)
        && let Some(i) = array_index_of(key)
    {
        // An accessor element of an Array: a DICTIONARY entry (M5-P2 ELEM2).
        dict_define(rt, o, i, pair, attrs);
        return Ok(true);
    }
    props::own_define(rt, o, key, pair, attrs);
    Ok(true)
}

/// Adds the new own property `key` (the object is extensible). GC point.
fn add_own_property(cx: &mut Cx, o: u64, key: u64, v: u64, attrs: i32) -> Res<bool> {
    let rt = cx.rt();
    if is_array(o)
        && let Some(i) = array_index_of(key)
    {
        // §10.4.2.1 step 3: an index at or past `length` raises it (array_set and
        // dict_define do), so a non-writable `length` refuses it (step 3.b). A {w, e, c}
        // element takes the dense path (or DICTIONARY past the dense limit, H18-Q4); other
        // attributes are a DICTIONARY entry (H18-Q2).
        if i >= object::array_len(o) && !elements::length_writable(o) {
            return Ok(false);
        }
        if !(attrs == attr::DEFAULT && object::array_set(rt, o, i, v)) {
            // GC point: `o` and `v` live in Rust locals.
            dict_define(rt, o, i, v, attrs);
        }
        return Ok(true);
    } else if attrs == attr::DEFAULT
        && elements::plain_elements_kind(kind::cell_kind(o))
        && object::has_elements_cell(o)
        && let Some(i) = array_index_of(key)
        && object::element_set(rt, o, i, v)
    {
        // A new {w, e, c} index key of a plain-elements object that has an elements cell
        // (H18-Q3; without one, a string key takes the key path, as in `define`); an index
        // too far for its elements store stays a stored property.
        return Ok(true);
    }
    props::own_define(rt, o, key, v, attrs);
    Ok(true)
}

/// ArraySetLength (§10.4.2.4) for `[[DefineOwnProperty]]("length", desc)`, and the
/// OrdinaryDefineOwnProperty of `length` without a value (step 1). The value is converted
/// first (ToUint32, then ToNumber: a RangeError if they differ), then checked against the
/// current `length` {w, e: false, c: false}. A shorter length deletes the elements from the
/// highest index down; a non-configurable one stops it (`length` = its index + 1) and the
/// result is false. `writable: false` is applied after the deletes (step 13's deferral);
/// it moves the array to DICTIONARY, whose cell holds the flag (M5-P2 ELEM2). GC point.
fn array_define_length(cx: &mut Cx, a: u64, desc: Desc) -> Res<bool> {
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
    let rt = cx.rt();
    let old = object::array_len(a);
    // ValidateAndApplyPropertyDescriptor against {old, w, e: false, c: false}.
    if desc.configurable == Some(true) || desc.enumerable == Some(true) {
        return Ok(false);
    }
    if !elements::length_writable(a) {
        // Steps 10-11: nothing may change but the same value.
        return Ok(desc.writable != Some(true) && new_len.is_none_or(|n| n == old));
    }
    let ok = match new_len {
        Some(n) => object::array_set_length(rt, a, n),
        None => true,
    };
    if desc.writable == Some(false) {
        // GC point: `a` lives in a Rust local.
        elements::set_length_readonly(rt, a);
    }
    Ok(ok)
}

/// `o.[[OwnPropertyKeys]]()` (§10.1.11.1 OrdinaryOwnPropertyKeys, §10.4.2 arrays, §10.4.3.3
/// String wrappers) as a new array: array indices ascending, then strings in creation order,
/// then symbols in creation order. The exotic properties come where the spec creates them:
/// `length` of an array or String wrapper first among the strings; a constructor's lazy
/// `prototype` after its `length` and `name`. A routed kind's hook
/// (`exotic::own_property_keys`) runs first. Not for a proxy (a debug assert;
/// [`try_own_property_keys`] takes one). GC point.
pub(crate) fn own_property_keys(cx: &mut Cx, o: u64) -> u64 {
    debug_assert!(is_object(o));
    if let Some(keys) = exotic::own_property_keys(cx, o) {
        return keys;
    }
    ordinary_own_property_keys(cx, o)
}

/// `o.[[OwnPropertyKeys]]()` for any object: a proxy asks its own (`proxy.rs`), every other
/// kind [`own_property_keys`]. GC point.
pub(crate) fn try_own_property_keys(cx: &mut Cx, o: u64) -> Res<u64> {
    if crate::proxy::is_proxy(o) {
        return crate::proxy::own_property_keys(cx, o);
    }
    Ok(own_property_keys(cx, o))
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
    if let Some(d) = elements::dict_of(o) {
        indices.extend(elements::dict_entries(d).into_iter().map(|(i, _)| i));
    } else if is_array(o) || elements::plain_elements_kind(kind::cell_kind(o)) {
        indices.extend(object::element_indices(o));
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
//   routed kind (ARGUMENTS, MODULE_NAMESPACE,
//                PROXY, PROXY_CALLABLE)          -> Unknown: its hooks, with the key
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
    /// Too many stored keys to scan, or a routed kind (ARGUMENTS, MODULE_NAMESPACE, a proxy)
    /// whose hooks decide: ask this object with the key.
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
/// MODULE_NAMESPACE, PROXY, PROXY_CALLABLE) always answers `Unknown`: its hooks decide, with
/// the key (H31 fast-path row 3: only routed kinds answer an index through hooks). A new
/// exotic kind with integer keys joins `exotic::is_routed` or adds its case here. Not a GC
/// point.
fn own_index(o: u64, i: u64, text: &[u8]) -> OwnIndex {
    let k = kind::cell_kind(o);
    if exotic::is_routed(k) {
        return OwnIndex::Unknown;
    }
    let index = array_index_u64(i);
    if let Some(n) = index {
        if k == kind::ARRAY.get()
            && let Some(d) = elements::dict_of(o)
        {
            // A DICTIONARY array (M5-P2 ELEM2): its entry, or none (an Array keeps no index
            // key in its property storage).
            return match elements::dict_get(d, n) {
                Some((v, a)) => OwnIndex::Stored(v, a),
                None => OwnIndex::Absent,
            };
        }
        if k == kind::ARRAY.get() || elements::plain_elements_kind(k) {
            if let Some(v) = object::element_get(o, n) {
                return OwnIndex::Stored(v, attr::DEFAULT);
            }
        } else if k == kind::PRIMITIVE_WRAPPER.get()
            && let Some(s) = wrapper::wrapper_value(o).filter(|&p| is_string(p))
            && (n as usize) < object::str_len(s)
        {
            return OwnIndex::WrapperChar(s);
        }
        // An array-index key that is a stored property moves a fast object to dictionary mode
        // (OM-10, `shape::add`); elements were read above.
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

/// HasProperty(O, ! ToString(𝔽(i))) (§7.3.12) on the object `o`, for a chain with no proxy
/// ([`has_property`]'s debug assert; [`try_has_index`] takes one). No atom for an element or
/// a missing index. GC point only on the key path.
// V9-A21 keeps the ordinary form; since `Cx::has_index` asks [`try_has_index`] (M4-P4 fixer
// B, Codex PROXY1 P-3) only the tests call it.
#[cfg_attr(not(test), allow(dead_code))]
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

/// [`has_index`] for any object: a proxy on the chain answers `Unknown` in `own_index` (a
/// routed kind), and the key path asks it ([`try_has_property`]). GC point on the key path.
pub(crate) fn try_has_index(cx: &mut Cx, o: u64, i: u64) -> Res<bool> {
    debug_assert!(is_object(o));
    if i <= MAX_SAFE_INDEX {
        let mut buf = [0u8; 20];
        let text = index_digits(i, &mut buf);
        match find_index(cx.rt(), o, i, text) {
            (_, OwnIndex::Stored(..) | OwnIndex::WrapperChar(_)) => return Ok(true),
            (_, OwnIndex::Absent) => return Ok(false),
            (cur, OwnIndex::Unknown) => {
                // GC point: `cur` lives in a Rust local.
                let key = index_key_of(cx.rt(), i);
                return try_has_property(cx, cur, key);
            }
        }
    }
    let key = index_key_of(cx.rt(), i);
    try_has_property(cx, o, key)
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
    if elements::plain_elements_kind(kind::cell_kind(o))
        && let Some(n) = array_index_u64(i)
        && plain_store(cx.rt(), o, n, v)
    {
        return Ok(true);
    }
    // GC point: `o` and `v` live in Rust locals.
    let key = index_key_of(cx.rt(), i);
    ordinary_set(cx, o, key, v)
}

/// CreateDataPropertyOrThrow(O, ! ToString(𝔽(i)), v) (§7.3.7) on the object `o`. A dense
/// array element is stored without a key (as `define_keyed`); else the key path. GC point.
pub(crate) fn define_index(cx: &mut Cx, o: u64, i: u64, v: u64) -> Res<()> {
    debug_assert!(is_object(o));
    if is_array(o)
        && let Some(n) = array_index_u64(i)
        // An existing element is {w, e, c}; a new one needs an extensible array.
        && (object::array_get(o, n).is_some() || is_extensible(o))
        && object::array_set(cx.rt(), o, n, v)
    {
        return Ok(());
    }
    if elements::plain_elements_kind(kind::cell_kind(o))
        && let Some(n) = array_index_u64(i)
        && plain_define(cx.rt(), o, n, v)
    {
        return Ok(());
    }
    // GC point: `o` and `v` live in Rust locals.
    let key = index_key_of(cx.rt(), i);
    define(cx, o, key, v)
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
        if elements::plain_elements_kind(kind::cell_kind(o))
            && let Some(n) = array_index_u64(i)
            && object::element_get(o, n).is_some()
        {
            object::element_set(cx.rt(), o, n, crate::frame::HOLE);
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
