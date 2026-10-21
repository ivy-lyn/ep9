//! Primitive wrapper objects (§10.4.3 String exotic objects; Number, Boolean, Symbol
//! wrappers): kind PRIMITIVE_WRAPPER through the fixed-field API. Week-4 stub; P2 builds
//! them.
#![allow(dead_code)] // wk4-stub

use crate::object;
use crate::runtime::Runtime;

/// A new wrapper of the primitive `value` with prototype `proto` (a plain object until
/// wrappers exist). GC point.
pub(crate) fn new_wrapper(rt: &Runtime, proto: u64, _value: u64) -> u64 {
    object::new_object(rt, proto)
}

/// The `[[PrimitiveValue]]` of the wrapper `o`, or `None` if `o` is not a wrapper.
pub(crate) fn wrapper_value(_o: u64) -> Option<u64> {
    None
}

/// StringGetOwnProperty (§10.4.3.5) of a String wrapper `o`: (value, attributes) for
/// `length` and the index keys, `None` otherwise.
pub(crate) fn string_own_property(_rt: &Runtime, _o: u64, _key: u64) -> Option<(u64, i32)> {
    None
}
