//! Primitive wrapper objects: String exotic objects (§10.4.3) and the Number, Boolean and
//! Symbol objects (§21.1.4, §20.3.4, §20.4.4). Kind PRIMITIVE_WRAPPER, one fixed field (the
//! `[[StringData]]`, `[[NumberData]]`, `[[BooleanData]]` or `[[SymbolData]]`) through the
//! fixed-field API of object.rs, so no offset is hard-coded here (H17 moves the header).
//!
//! ```text
//! ToObject(v) (conv::to_object)  string  -> new_wrapper(%String.prototype%, v)
//!                                number  -> new_wrapper(%Number.prototype%, v)
//!                                boolean -> new_wrapper(%Boolean.prototype%, v)
//!                                symbol  -> new_wrapper(%Symbol.prototype%, v)  (symbol::prototype)
//! own properties of a String wrapper (property.rs, exotic.rs):
//!   string_own_property      "length" {v: len, w: f, e: f, c: f}; index i < len {v: s[i], w: f, e: t, c: f}
//!   string_has_own_property  the same test without the value (allocates nothing)
//! ```

use crate::frame::{NULL, UNDEFINED};
use crate::key::{array_index_of, key_eq};
use crate::kinds::{self as kind, is_object, is_string, is_wrapper};
use crate::object::{self, attr, fixed};
use crate::runtime::{Atom, Runtime};
use crate::string;

/// Index of the primitive value among the wrapper's fixed fields.
const VALUE: usize = 0;

/// A new wrapper object of the primitive `value` (a string, number, boolean or symbol) with
/// prototype `proto` (an object or null). GC point: `value` and `proto` live in Rust locals
/// (conservative scan) across the allocation.
pub(crate) fn new_wrapper(rt: &Runtime, proto: u64, value: u64) -> u64 {
    debug_assert!(!is_object(value) && value != UNDEFINED && value != NULL);
    let o = object::new_object_with_fields(rt, kind::PRIMITIVE_WRAPPER, proto, fixed::WRAPPER);
    object::fixed_set(rt, o, VALUE, value);
    o
}

/// The `[[PrimitiveValue]]` of the wrapper `o`, or `None` if `o` is not a wrapper.
pub(crate) fn wrapper_value(o: u64) -> Option<u64> {
    is_wrapper(o).then(|| object::fixed_get(o, VALUE))
}

/// The wrapped string of `o` if it is a String wrapper.
fn string_data(o: u64) -> Option<u64> {
    wrapper_value(o).filter(|&v| is_string(v))
}

/// Where `key` falls in the String wrapper data `s`.
enum StringKey {
    Length,
    Index(u32),
}

fn string_key(rt: &Runtime, s: u64, key: u64) -> Option<StringKey> {
    if let Some(i) = array_index_of(key) {
        // §10.4.3.5 steps 2-8: a canonical numeric index below the length. `array_index_of`
        // accepts only canonical index strings ("1", not "01" or "-0"); larger integers are
        // past every string length (at most 2^29 - 24).
        return ((i as usize) < string::str_len(s)).then_some(StringKey::Index(i));
    }
    key_eq(key, rt.atom(Atom::Length)).then_some(StringKey::Length)
}

/// StringGetOwnProperty (§10.4.3.5) and the `length` property of a String exotic object
/// (§10.4.3.4 StringCreate step 9) for the wrapper `o`: (value, [`attr`] bits) for `length`
/// ({w: false, e: false, c: false}) and the indices below it ({w: false, e: true, c: false});
/// `None` for any other key or object. GC point (the one-character string of an index).
pub(crate) fn string_own_property(rt: &Runtime, o: u64, key: u64) -> Option<(u64, i32)> {
    let s = string_data(o)?;
    Some(match string_key(rt, s, key)? {
        StringKey::Length => (crate::conv::num(string::str_len(s) as f64), 0),
        StringKey::Index(i) => (string::char_at(rt, s, i as usize), attr::ENUMERABLE),
    })
}

/// True if [`string_own_property`] has a property for `key`. Allocates nothing.
#[allow(dead_code)] // callers: P2-OBJ (HasOwnProperty, which allocates nothing)
pub(crate) fn string_has_own_property(rt: &Runtime, o: u64, key: u64) -> bool {
    string_data(o).is_some_and(|s| string_key(rt, s, key).is_some())
}
