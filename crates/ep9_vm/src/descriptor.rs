//! Property descriptors (§6.2.6): every field optional, as ToPropertyDescriptor gives them.
//! Week-4 stub; P1 and P3 use it.
#![allow(dead_code)] // wk4-stub

/// A property descriptor. `value`, `get` and `set` hold Value bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Desc {
    pub value: Option<u64>,
    pub get: Option<u64>,
    pub set: Option<u64>,
    pub writable: Option<bool>,
    pub enumerable: Option<bool>,
    pub configurable: Option<bool>,
}
