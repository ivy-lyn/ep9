//! Class fields, static blocks and private methods in the VM (owner P2-CLS-VM): runtime ids
//! 24-25 (`SetClassFieldsInitializer`, `RunClassFieldsInitializer`, ABI 3), which
//! `slow/runtime.rs` routes here, and the "set the function name" bit (bit 0) of
//! `DefineKeyedOwnProperty` and `DefineKeyedOwnPropertyInLiteral`. Id 23 `CreatePrivateName`
//! and the private keyed paths are in `slow/private.rs`.
//!
//! ```text
//! class C { x = 1; #y = 2 }        the front end compiles the instance fields (and the brand
//!                                  adds of private methods) into one initializer function
//! DefineClass ...
//! SetClassFieldsInitializer(C, init)   C[class-fields key] = init (an own entry, attrs 0),
//!                                      init.[[HomeObject]] = C.prototype (MakeMethod)
//! base ctor entry / after super():
//! RunClassFieldsInitializer(C, this)   C's own entry only; Call(init, this, [])
//!                                      (InitializeInstanceElements, §7.3.34)
//! ```
//!
//! The initializer is stored on the constructor under the class-fields key, a VM-internal
//! private name (`symbol::class_fields_key`; C8): the FUNCTION layout has no free field, and a
//! private key is never listed by `[[OwnPropertyKeys]]` or for-in, so `Object.getOwnPropertyNames`
//! and `Reflect.ownKeys` do not show it. The lookup is own-only, so a subclass with no fields
//! never runs its parent's initializer through the constructor's prototype chain.

use crate::abi::{self, JsObject, JsValue, Local};
use crate::conv::{self, Res};
use crate::error::throw_error;
use crate::frame::UNDEFINED;
use crate::interp::Cx;
use crate::object::{self, attr, is_function, is_object};
use crate::props;
use crate::runtime::{Atom, ErrorKind};
use crate::symbol;

use super::private::SET_NAME;

/// `SetClassFieldsInitializer(constructor, initializer)` (runtime id 24): stores the instance
/// members initializer on `constructor` under the class-fields key, and makes it a method of
/// `constructor.prototype` (ClassFieldDefinitionEvaluation: MakeMethod(initializer,
/// homeObject), §15.7.10), so `super.x` in a field initializer reads the parent prototype. An
/// undefined `initializer` stores nothing. GC point.
pub(crate) fn set_class_fields_initializer(
    cx: &mut Cx,
    constructor: u64,
    initializer: u64,
) -> Res<u64> {
    if initializer == UNDEFINED {
        return Ok(UNDEFINED);
    }
    if !is_function(constructor) || !is_function(initializer) {
        return Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: SetClassFieldsInitializer needs two functions",
        ));
    }
    let rt = cx.rt();
    // DefineClass made `prototype` an own data property of the constructor.
    if let Some((proto, a)) = props::own_get(constructor, rt.atom(Atom::Prototype))
        && a & attr::ACCESSOR == 0
        && is_object(proto)
    {
        object::set_home_object(rt, initializer, proto);
    }
    let key = symbol::class_fields_key(rt);
    // GC point (a new shape or table): every value lives in a Rust local (CSS). A second call
    // for one constructor replaces the entry.
    if !props::own_replace(rt, constructor, key, initializer) {
        props::own_add(rt, constructor, key, initializer, 0);
    }
    Ok(UNDEFINED)
}

/// The initializer that [`set_class_fields_initializer`] stored on `constructor` (an own
/// entry only), or None. Not a GC point.
pub(crate) fn class_fields_initializer(cx: &Cx, constructor: u64) -> Option<u64> {
    if !is_object(constructor) {
        return None;
    }
    let key = symbol::class_fields_key(cx.rt());
    props::own_get(constructor, key).map(|(f, _)| f)
}

/// `RunClassFieldsInitializer(constructor, receiver)` (runtime id 25): InitializeInstanceElements
/// (§7.3.34) as one call: `Call(initializer, receiver)` for the initializer stored on
/// `constructor`, nothing if there is none. Returns undefined. GC point (runs JS).
pub(crate) fn run_class_fields_initializer(
    cx: &mut Cx,
    constructor: u64,
    receiver: u64,
) -> Res<u64> {
    let Some(f) = class_fields_initializer(cx, constructor) else {
        return Ok(UNDEFINED);
    };
    crate::call::call_function(cx, f, receiver, &[])?;
    Ok(UNDEFINED)
}

/// The public-key arm of `DefineKeyedOwnProperty` and `DefineKeyedOwnPropertyInLiteral`
/// (`slow/property.rs`): CreateDataPropertyOrThrow of `o[k] = v`. With flag bit 0
/// ([`SET_NAME`]) the front end saw an anonymous function definition (a computed object
/// literal key, a computed class field key): ToPropertyKey(k) once, then SetFunctionName(v,
/// key) (§10.2.9) by [`name_anonymous_function`], then the define. GC point.
pub(crate) fn define_keyed_own(cx: &mut Cx, o: u64, k: u64, v: u64, flags: u8) -> Res<()> {
    if flags & SET_NAME == 0 || !is_object(o) {
        return super::property::define_checked(cx, o, k, v, true);
    }
    let key = conv::to_property_key(cx, k)?;
    // GC point: `o`, `key` and `v` live in Rust locals (CSS).
    name_anonymous_function(cx, v, key)?;
    super::property::define_checked(cx, o, key, v, true)
}

/// SetFunctionName(v, name) (§10.2.9) for NamedEvaluation of an anonymous function
/// definition whose name comes from a runtime key: `name` is a property key (a symbol gives
/// `"[description]"`) or the description string of a private name. It names only a bytecode
/// function that has no name from its unit and whose own `name` is still the initial `""`
/// {w: false, e: false, c: true}: a class with a `static name` member, or one whose static
/// block deleted `name`, keeps what its definition made (ClassDefinitionEvaluation names the
/// class before its static elements run). GC point.
pub(crate) fn name_anonymous_function(cx: &mut Cx, v: u64, name: u64) -> Res<()> {
    if !is_function(v) {
        return Ok(());
    }
    let fi = object::function_info(v);
    let anonymous = fi
        .loaded_unit()
        .is_some_and(|u| u.unit.functions[fi.function_id as usize].name.is_none());
    if !anonymous {
        return Ok(());
    }
    let rt = cx.rt();
    let empty = rt.atom(Atom::Empty);
    match props::own_get(v, rt.atom(Atom::Name)) {
        Some((n, a)) if n == empty && a == attr::CONFIGURABLE => {}
        _ => return Ok(()),
    }
    let mut acx = abi::Cx::new(cx);
    acx.set_function_name(
        Local::<JsObject>::from_bits(v),
        Local::<JsValue>::from_bits(name),
        None,
    )
}
