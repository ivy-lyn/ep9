//! Property keys (H19 PropertyKey): ToPropertyKey, key equality, hashing, interning.
//!
//! A key is the Value bits of a string or a symbol. There are no number keys: an index stays
//! a string atom ("1"), and its hash field caches the index (W4-NO-NUMBER-KEYS). A symbol is
//! its own key: one cell per symbol, so it always compares by bits.
//!
//! ```text
//! producer                     key it hands on
//! ToPropertyKey (any value) -> a symbol as it is; else flatten, then the atom of the string
//! unit strings              -> atoms (interned at unit load, unit.rs)
//! Rust text                 -> atoms::intern_str
//! define paths              -> intern_key (P2-OBJ calls it; stored keys are always atoms)
//!
//! lookup: probe(key) once -> Bits (atom or symbol): compare bits only
//!                         -> Content (any other string): one content scan, debug counter + 1
//! hash:   key_hash(key)    -> a string's 30-bit table hash, or a symbol's 30-bit identity hash
//! ```

use crate::atoms;
use crate::conv::{Hint, Res, describe, to_primitive, to_string};
use crate::interp::Cx;
use crate::kinds::{is_string, is_symbol};
use crate::runtime::Runtime;
use crate::{string, symbol};

/// A property key: the Value bits of a string or a symbol.
pub(crate) type PropertyKey = u64;

/// How a lookup compares a probe key with stored keys (classified once per lookup).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Probe {
    /// An atom or a symbol: compare bits only.
    Bits,
    /// Any other string: one content compare.
    Content,
}

/// Property-key equality: the same cell, or two strings with equal code units. Two distinct
/// atoms are never equal, so only a pair with a non-atom string looks at content (and bumps
/// the debug counter, W4-KEY-EQ).
#[inline]
pub(crate) fn key_eq(a: PropertyKey, b: PropertyKey) -> bool {
    if a == b {
        return true;
    }
    if !(is_string(a) && is_string(b)) || (string::is_atom(a) && string::is_atom(b)) {
        return false;
    }
    atoms::note_non_atom_compare();
    string::equals(a, b)
}

/// ToPropertyKey (§7.1.19): ToPrimitive(b, string); a symbol is the key, anything else
/// becomes the atom of its ToString. An index such as "1" stays a string atom.
pub(crate) fn to_property_key(cx: &mut Cx, b: u64) -> Res<PropertyKey> {
    if string::is_atom(b) || is_symbol(b) {
        return Ok(b);
    }
    let s = if is_string(b) {
        b
    } else {
        let p = to_primitive(cx, b, Hint::String)?;
        if is_symbol(p) {
            return Ok(p);
        }
        to_string(cx, p)?
    };
    let s = string::flatten(cx, s);
    Ok(intern_key(cx.rt(), s))
}

/// The array index that `key` names, if it is a canonical index string ("0" .. "4294967294").
/// An atom answers from its cached hash field.
pub(crate) fn array_index_of(key: PropertyKey) -> Option<u32> {
    if is_string(key) {
        string::array_index(key)
    } else {
        None
    }
}

/// True if `key` is an array index.
#[allow(dead_code)] // callers: P2-OBJ (OM-10 index keys)
pub(crate) fn is_index_key(key: PropertyKey) -> bool {
    array_index_of(key).is_some()
}

/// True if `key` is a symbol.
pub(crate) fn is_symbol_key(key: PropertyKey) -> bool {
    is_symbol(key)
}

/// True if `key` is a valid property key (a string or a symbol).
pub(crate) fn is_property_key(key: PropertyKey) -> bool {
    is_string(key) || is_symbol_key(key)
}

/// The text of `key` for error messages (never runs JS): a string's text, or a symbol's
/// `Symbol(desc)` (SymbolDescriptiveString).
pub(crate) fn key_text(_rt: &Runtime, key: PropertyKey) -> String {
    if is_string(key) {
        string::to_rust_string(key)
    } else if is_symbol(key) {
        symbol::descriptive_text(key)
    } else {
        describe(key)
    }
}

/// The key to store for `key`: the atom of a string key (H19); a symbol as it is. Not a GC
/// point.
pub(crate) fn intern_key(rt: &Runtime, key: PropertyKey) -> PropertyKey {
    if is_string(key) {
        atoms::intern(rt, key)
    } else {
        key
    }
}

/// Classifies a probe key once per lookup (I47): an atom or a symbol compares by bits; any
/// other string needs a content compare, which the debug counter records (W4-KEY-EQ).
#[allow(dead_code)] // callers: shape lookups (P1-SHAPE), props lookups (P2-OBJ)
pub(crate) fn probe(key: PropertyKey) -> Probe {
    if string::is_atom(key) || is_symbol(key) {
        Probe::Bits
    } else {
        atoms::note_non_atom_compare();
        Probe::Content
    }
}

/// The 30-bit hash of a key: a string's table hash (its cached hash field, computed on first
/// use) or a symbol's identity hash (PRNG, never the address; H12). Seeded per process:
/// never in bytecode, units or property order. Not a GC point.
#[allow(dead_code)] // callers: DESC_INDEX (P1-SHAPE)
pub(crate) fn key_hash(key: PropertyKey) -> u32 {
    if is_symbol(key) {
        return symbol::hash(key);
    }
    debug_assert!(is_string(key), "a property key is a string or a symbol");
    string::hash(key)
}
