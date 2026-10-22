//! Own properties that do not live in an object's property storage (`props.rs`): the
//! exotic ones of arrays (§10.4.2) and String wrappers (§10.4.3), and the lazy `prototype` of
//! constructor functions. `property.rs` asks these hooks before the ordinary storage.
//!
//! | Receiver | Own properties seen here | Attributes |
//! |---|---|---|
//! | array | `length`; array-index keys below the dense limit (elements) | {w}; {w, e, c} |
//! | String wrapper | `length`; indices below the length (`wrapper::string_own_property`) | {}; {e} |
//! | constructor or generator function | `prototype` until first use, then an ordinary property | {w} |
//! | arguments object (kind ARGUMENTS) | routed to `arguments.rs` (M3-P2 VM-ARGS) | - |
//! | module namespace (kind MODULE_NAMESPACE) | routed to `module.rs` (M3-P2 MODULE) | - |
//!
//! Function `length` and `name` are ordinary own data properties (`object::new_function`).
//! The lazy `prototype` follows V8, which also creates it on first access.
//!
//! Routing (OBJECT_LAYOUT 3, P0-D7): the two exotic kinds of month 3 replace internal methods
//! (§10.4.4, §10.4.6). Each hook here ([`get`], [`own_property`], [`own_attrs`]) and in
//! `property.rs` ([`set`], [`define_own_property`], [`delete`], [`own_property_keys`]) makes
//! one kind test ([`is_routed`]) and sends those kinds to their file, out of line. `[[Set]]`
//! is routed at its start and at each routed prototype of its walk (§10.1.9.2 step 1.b). A
//! routed hook that answers `None` means "ordinary": in M3-P0 every routed hook is a stub
//! that does.

use crate::conv::{Res, num};
use crate::descriptor::Desc;
use crate::generator::{self, LazyPrototype};
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

/// True if the function `f` has a lazy `prototype` (`generator::lazy_prototype_kind`: a
/// constructor, later also a generator function) that is still lazy (not in its storage),
/// and `key` is `prototype`. Not a GC point.
#[inline]
pub(crate) fn is_lazy_prototype(rt: &Runtime, f: u64, key: u64, how: Probe) -> bool {
    generator::lazy_prototype_kind(object::function_info(f)).is_some()
        && is_key(rt, key, how, Atom::Prototype)
        && props::own_get_with(f, key, how).is_none()
}

/// The `prototype` object of the function `f`, created on first use
/// (`generator::lazy_prototype_kind`): for a constructor, with a `constructor` link back to
/// `f` (§10.2.5 MakeConstructor); for a generator function, an object that inherits from the
/// kind's intrinsic (§15.5.4). Both are {w: true, e: false, c: false}. Also made before any
/// later key is added to `f` (`props::own_add`), so it keeps its creation-order place among
/// the keys. GC point.
pub(crate) fn function_prototype(rt: &Runtime, f: u64) -> u64 {
    let key = rt.atom(Atom::Prototype);
    if let Some((v, _)) = props::own_get(f, key) {
        return v;
    }
    let kind = generator::lazy_prototype_kind(object::function_info(f));
    // GC points: `f` and `p` live in Rust locals.
    let p = match kind {
        Some(LazyPrototype::Instances(id)) => {
            let parent = crate::builtins::intrinsic(rt, id);
            debug_assert!(kind::is_object(parent), "{} is not installed", id.name());
            object::new_object(rt, parent)
        }
        Some(LazyPrototype::Ordinary) | None => {
            let p = object::new_plain_object(rt);
            props::own_define(rt, p, rt.atom(Atom::Constructor), f, attr::HIDDEN);
            p
        }
    };
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
    if is_routed(k) {
        return routed_get(cx, k, o, key, how);
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
    /// An own property that a routed kind's hook reports (`arguments.rs`, `module.rs`). Its
    /// `[[DefineOwnProperty]]` hook applies a change to it, never the ordinary apply step.
    Routed,
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
    if is_routed(k) {
        return routed_own_property(cx, k, o, key, how).map(|(v, a)| (v, a, Exotic::Routed));
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
    if is_routed(k) {
        return routed_own_attrs(rt, k, o, key, how);
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

// ---- routing to arguments.rs and module.rs (OBJECT_LAYOUT 3, P0-D7) --------------------------

/// True for the kinds whose internal methods are routed: ARGUMENTS (25) and
/// MODULE_NAMESPACE (26). One compare on adjacent ids.
#[inline(always)]
pub(crate) fn is_routed(k: u8) -> bool {
    k == kind::ARGUMENTS.get() || k == kind::MODULE_NAMESPACE.get()
}

#[inline(never)]
fn routed_get(cx: &mut Cx, k: u8, o: u64, key: u64, how: Probe) -> Option<u64> {
    if k == kind::ARGUMENTS.get() {
        crate::arguments::get(cx, o, key, how)
    } else {
        crate::module::get(cx, o, key, how)
    }
}

#[inline(never)]
fn routed_own_property(cx: &mut Cx, k: u8, o: u64, key: u64, how: Probe) -> Option<(u64, i32)> {
    if k == kind::ARGUMENTS.get() {
        crate::arguments::own_property(cx, o, key, how)
    } else {
        crate::module::own_property(cx, o, key, how)
    }
}

#[inline(never)]
fn routed_own_attrs(rt: &Runtime, k: u8, o: u64, key: u64, how: Probe) -> Option<i32> {
    if k == kind::ARGUMENTS.get() {
        crate::arguments::own_attrs(rt, o, key, how)
    } else {
        crate::module::own_attrs(rt, o, key, how)
    }
}

/// The `[[Set]]` hook of `o` (`property::set_with_receiver`): `None` = OrdinarySet.
#[inline(always)]
pub(crate) fn set(cx: &mut Cx, o: u64, key: u64, v: u64, receiver: u64) -> Option<Res<bool>> {
    let k = kind::cell_kind(o);
    if is_routed(k) {
        routed_set(cx, k, o, key, v, receiver)
    } else {
        None
    }
}

#[inline(never)]
fn routed_set(cx: &mut Cx, k: u8, o: u64, key: u64, v: u64, receiver: u64) -> Option<Res<bool>> {
    if k == kind::ARGUMENTS.get() {
        crate::arguments::set(cx, o, key, v, receiver)
    } else {
        crate::module::set(cx, o, key, v, receiver)
    }
}

/// The `[[DefineOwnProperty]]` hook of `o` (`property::define_own_property`): `None` =
/// OrdinaryDefineOwnProperty.
#[inline(always)]
pub(crate) fn define_own_property(cx: &mut Cx, o: u64, key: u64, desc: Desc) -> Option<Res<bool>> {
    let k = kind::cell_kind(o);
    if is_routed(k) {
        routed_define_own_property(cx, k, o, key, desc)
    } else {
        None
    }
}

#[inline(never)]
fn routed_define_own_property(
    cx: &mut Cx,
    k: u8,
    o: u64,
    key: u64,
    desc: Desc,
) -> Option<Res<bool>> {
    if k == kind::ARGUMENTS.get() {
        crate::arguments::define_own_property(cx, o, key, desc)
    } else {
        crate::module::define_own_property(cx, o, key, desc)
    }
}

/// The `[[Delete]]` hook of `o` (`property::delete`): `None` = OrdinaryDelete.
#[inline(always)]
pub(crate) fn delete(cx: &mut Cx, o: u64, key: u64) -> Option<Res<bool>> {
    let k = kind::cell_kind(o);
    if is_routed(k) {
        routed_delete(cx, k, o, key)
    } else {
        None
    }
}

#[inline(never)]
fn routed_delete(cx: &mut Cx, k: u8, o: u64, key: u64) -> Option<Res<bool>> {
    if k == kind::ARGUMENTS.get() {
        crate::arguments::delete(cx, o, key)
    } else {
        crate::module::delete(cx, o, key)
    }
}

/// The `[[OwnPropertyKeys]]` hook of `o` (`property::own_property_keys`): `None` =
/// OrdinaryOwnPropertyKeys.
#[inline(always)]
pub(crate) fn own_property_keys(cx: &mut Cx, o: u64) -> Option<u64> {
    let k = kind::cell_kind(o);
    if is_routed(k) {
        routed_own_property_keys(cx, k, o)
    } else {
        None
    }
}

#[inline(never)]
fn routed_own_property_keys(cx: &mut Cx, k: u8, o: u64) -> Option<u64> {
    if k == kind::ARGUMENTS.get() {
        crate::arguments::own_property_keys(cx, o)
    } else {
        crate::module::own_property_keys(cx, o)
    }
}

/// Which routed hooks ran on this thread (test builds only; a no-op otherwise): the routing
/// tests (V3-A13) check that each hook reaches its stub. The stubs call [`probe::note`]; their
/// owners drop the calls when they fill them.
pub(crate) mod probe {
    #[cfg(test)]
    thread_local! {
        static HITS: core::cell::RefCell<Vec<&'static str>> =
            const { core::cell::RefCell::new(Vec::new()) };
    }

    /// Records a hook (test builds); nothing otherwise.
    #[inline(always)]
    pub(crate) fn note(what: &'static str) {
        #[cfg(test)]
        HITS.with(|h| h.borrow_mut().push(what));
        let _ = what;
    }

    /// The hooks recorded on this thread since the last call, in order.
    #[cfg(test)]
    pub(crate) fn take() -> Vec<&'static str> {
        HITS.with(|h| core::mem::take(&mut *h.borrow_mut()))
    }
}
