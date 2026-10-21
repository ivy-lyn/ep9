//! Atoms (H19): the per-isolate table of interned strings. Week-4 stub; P1 builds the weak
//! table.
//!
//! GC rule [I41]: the table sits in an `Rc<RefCell<..>>` that the heap's weak source also
//! holds. Never hold a borrow of it across an allocation: allocate the string first, then
//! borrow and insert.
#![allow(dead_code)] // wk4-stub

use crate::runtime::{Atom, Runtime};
use crate::string;

/// The atom table of one isolate (`Runtime::atoms`).
#[derive(Debug, Default)]
pub(crate) struct AtomTable {}

/// The atom for the string `s` (identity until the table exists).
pub(crate) fn intern(_rt: &Runtime, s: u64) -> u64 {
    s
}

/// The atom for the text `s`. Every property key built from Rust text goes through here.
pub(crate) fn intern_str(rt: &Runtime, s: &str) -> u64 {
    string::new_string(rt, s)
}

/// Creates the static atoms (`Atom::ALL`) as isolate roots. Runs first in `realm::init`.
pub(crate) fn init_static(rt: &Runtime) {
    for &a in Atom::ALL {
        let s = string::new_string(rt, a.text());
        rt.set_atom(a, s);
    }
}
