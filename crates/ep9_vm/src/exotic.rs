//! Own properties that do not live in an object's property storage (`props.rs`): the
//! exotic ones of arrays (§10.4.2) and String wrappers (§10.4.3), and the lazy `prototype` of
//! constructor functions. `property.rs` asks these hooks before the ordinary storage.
//!
//! | Receiver | Own properties seen here | Attributes |
//! |---|---|---|
//! | array | `length`; array-index keys below the dense limit (elements) | {w}; {w, e, c} |
//! | String wrapper | `length`; indices below the length (`wrapper::string_own_property`) | {}; {e} |
//! | constructor function | `prototype` until first use, then an ordinary property | {w} |
//!
//! Function `length` and `name` are ordinary own data properties (`object::new_function`).
//! The lazy `prototype` follows V8, which also creates it on first access.

use crate::conv::num;
use crate::interp::Cx;
use crate::key::{Probe, array_index_of};
use crate::kinds as kind;
use crate::object::{self, attr};
use crate::props;
use crate::runtime::{Atom, Runtime};
use crate::wrapper;

/// True if `key` is the atom `a`. With a classified probe, an atom compares by bits only.
#[inline]
pub(crate) fn is_key(rt: &Runtime, key: u64, how: Probe, a: Atom) -> bool {
    let atom = rt.atom(a);
    match how {
        Probe::Bits => key == atom,
        Probe::Content => object::key_eq(key, atom),
    }
}

/// True if the function `f` is a constructor whose `prototype` is still lazy (not in its
/// storage) and `key` is `prototype`. Not a GC point.
#[inline]
pub(crate) fn is_lazy_prototype(rt: &Runtime, f: u64, key: u64, how: Probe) -> bool {
    object::function_info(f).is_constructor()
        && is_key(rt, key, how, Atom::Prototype)
        && props::own_get_with(f, key, how).is_none()
}

/// The `prototype` object of the constructor `f`, created on first use with a `constructor`
/// link back to `f` (§10.2.5 MakeConstructor: {w: true, e: false, c: false}). Also made
/// before any later key is added to `f` (`props::own_add`), so it keeps its creation-order
/// place among the keys. GC point.
pub(crate) fn function_prototype(rt: &Runtime, f: u64) -> u64 {
    let key = rt.atom(Atom::Prototype);
    if let Some((v, _)) = props::own_get(f, key) {
        return v;
    }
    // GC points: `f` and `p` live in Rust locals.
    let p = object::new_plain_object(rt);
    props::own_define(rt, p, rt.atom(Atom::Constructor), f, attr::HIDDEN);
    props::own_define(rt, f, key, p, attr::WRITABLE);
    p
}

/// The value of an exotic own property of `o` for a get: an array's `length` or element, a
/// String wrapper's `length` or character, or a constructor's lazy `prototype` (made here).
/// `None` if `key` names none of them. GC point (the lazy `prototype`). Always inlined into
/// the `[[Get]]` walks (`property::get_with_receiver`, `property::get_data_from`).
#[inline(always)]
pub(crate) fn get(cx: &mut Cx, o: u64, key: u64, how: Probe) -> Option<u64> {
    let k = kind::cell_kind(o);
    if k == kind::ARRAY.get() {
        if is_key(cx.rt(), key, how, Atom::Length) {
            return Some(num(f64::from(object::array_len(o))));
        }
        return array_index_of(key).and_then(|i| object::array_get(o, i));
    }
    if k == kind::FUNCTION.get() {
        if is_lazy_prototype(cx.rt(), o, key, how) {
            return Some(function_prototype(cx.rt(), o));
        }
        return None;
    }
    if k == kind::PRIMITIVE_WRAPPER.get() {
        return wrapper::string_own_property(cx.rt(), o, key).map(|(v, _)| v);
    }
    None
}

/// Where an exotic own property lives, for the apply step of `[[DefineOwnProperty]]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Exotic {
    /// A dense array element.
    Element(u32),
    /// A String wrapper's `length` or index (never writable or configurable).
    Wrapper,
}

/// The exotic own property `key` of `o` (array `length` excluded: the callers handle it):
/// (value, attribute bits, where). A constructor's lazy `prototype` is made here and then is
/// an ordinary property, so it gives `None`. GC point.
pub(crate) fn own_property(
    cx: &mut Cx,
    o: u64,
    key: u64,
    how: Probe,
) -> Option<(u64, i32, Exotic)> {
    let k = kind::cell_kind(o);
    if k == kind::ARRAY.get() {
        let i = array_index_of(key)?;
        return object::array_get(o, i).map(|v| (v, attr::DEFAULT, Exotic::Element(i)));
    }
    if k == kind::FUNCTION.get() {
        if is_lazy_prototype(cx.rt(), o, key, how) {
            function_prototype(cx.rt(), o);
        }
        return None;
    }
    if k == kind::PRIMITIVE_WRAPPER.get() {
        return wrapper::string_own_property(cx.rt(), o, key).map(|(v, a)| (v, a, Exotic::Wrapper));
    }
    None
}

/// The attributes of an exotic own property of `o`, without making anything: an array's
/// `length` or element, a String wrapper's `length` or index, a constructor's lazy
/// `prototype`. Not a GC point.
pub(crate) fn own_attrs(rt: &Runtime, o: u64, key: u64, how: Probe) -> Option<i32> {
    let k = kind::cell_kind(o);
    if k == kind::ARRAY.get() {
        if is_key(rt, key, how, Atom::Length) {
            return Some(attr::WRITABLE);
        }
        let i = array_index_of(key)?;
        return object::array_get(o, i).map(|_| attr::DEFAULT);
    }
    if k == kind::FUNCTION.get() {
        return is_lazy_prototype(rt, o, key, how).then_some(attr::WRITABLE);
    }
    if k == kind::PRIMITIVE_WRAPPER.get() {
        return wrapper::string_own_attrs(rt, o, key);
    }
    None
}

/// The lazy `prototype` key to list among the string keys of `o` (`[[OwnPropertyKeys]]`), if
/// `o` is a constructor whose `prototype` is not made yet. It goes after every stored string
/// key: adding a key makes it first (`props::own_add`), so all stored keys are older.
pub(crate) fn lazy_prototype_key(rt: &Runtime, o: u64) -> Option<u64> {
    let key = rt.atom(Atom::Prototype);
    (kind::is_function(o) && is_lazy_prototype(rt, o, key, Probe::Bits)).then_some(key)
}

/// The `length` key to list first among the string keys of an array or a String wrapper.
pub(crate) fn virtual_length_key(rt: &Runtime, o: u64) -> Option<u64> {
    let wrapped_string = wrapper::wrapper_value(o).is_some_and(kind::is_string);
    (kind::is_array(o) || wrapped_string).then(|| rt.atom(Atom::Length))
}
