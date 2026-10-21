//! Property keys (H19 PropertyKey): ToPropertyKey, key equality, hashing, interning.
//!
//! A key is the Value bits of a string (or, from P2, a symbol). There are no number keys: an
//! index stays a string ("1"), and H19 caches its index in the string's hash field.
//!
//! | Helper | Today | Later |
//! |---|---|---|
//! | [`intern_key`] | identity | the atom of the key (stored keys are always atoms) |
//! | [`probe`] | always [`Probe::Content`] | [`Probe::Bits`] for atoms and symbols |
//! | [`key_hash`] | FNV-1a over the code units | the stored hash |
//! | [`is_symbol_key`] | false | the symbol test |

use crate::conv::{Hint, Res, describe, to_primitive, to_string};
use crate::interp::Cx;
use crate::kinds::is_string;
use crate::runtime::Runtime;
use crate::string::str_view;

/// A property key: the Value bits of a string or a symbol.
#[allow(dead_code)] // wk4-stub
pub(crate) type PropertyKey = u64;

/// How a lookup compares a probe key with stored keys (classified once per lookup).
#[allow(dead_code)] // wk4-stub
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Probe {
    /// An atom or a symbol: compare bits only.
    Bits,
    /// Any other string: one content compare.
    Content,
}

/// Property-key equality: the same cell, or two strings with equal code units.
#[inline]
pub(crate) fn key_eq(a: u64, b: u64) -> bool {
    a == b || (is_string(a) && is_string(b) && str_view(a).eq(&str_view(b)))
}

/// ToPropertyKey (§7.1.19). Keys are strings until symbols exist.
pub(crate) fn to_property_key(cx: &mut Cx, b: u64) -> Res<u64> {
    if is_string(b) {
        return Ok(b);
    }
    let p = to_primitive(cx, b, Hint::String)?;
    to_string(cx, p)
}

/// The array index that `key` names, if it is a canonical index string ("0" .. "4294967294").
pub(crate) fn array_index_of(key: u64) -> Option<u32> {
    if is_string(key) {
        str_view(key).array_index()
    } else {
        None
    }
}

/// True if `key` is an array index.
#[allow(dead_code)] // wk4-stub
pub(crate) fn is_index_key(key: u64) -> bool {
    array_index_of(key).is_some()
}

/// True if `key` is a symbol.
pub(crate) fn is_symbol_key(_key: u64) -> bool {
    false
}

/// True if `key` is a valid property key (a string or a symbol).
pub(crate) fn is_property_key(key: u64) -> bool {
    is_string(key) || is_symbol_key(key)
}

/// The text of `key` for error messages (never runs JS).
pub(crate) fn key_text(_rt: &Runtime, key: u64) -> String {
    if is_string(key) {
        str_view(key).to_string_lossy()
    } else {
        describe(key)
    }
}

/// The key to store for `key` (the atom of the key, H19).
#[allow(dead_code)] // wk4-stub
pub(crate) fn intern_key(_rt: &Runtime, key: u64) -> u64 {
    key
}

/// Classifies a probe key once per lookup.
#[allow(dead_code)] // wk4-stub
pub(crate) fn probe(_key: u64) -> Probe {
    Probe::Content
}

/// A hash of the key's content: 32-bit FNV-1a over the UTF-16 code units, so the Latin-1
/// and UTF-16 forms of one text hash alike. Never in bytecode, units or property order.
#[allow(dead_code)] // wk4-stub
pub(crate) fn key_hash(key: u64) -> u32 {
    debug_assert!(is_string(key));
    let s = str_view(key);
    let mut h: u32 = 0x811c_9dc5;
    for i in 0..s.len() {
        for b in s.at(i).to_le_bytes() {
            h ^= u32::from(b);
            h = h.wrapping_mul(0x0100_0193);
        }
    }
    h
}
