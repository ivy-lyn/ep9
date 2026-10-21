//! gc-lint case: a type that holds a `Local` and puts its type parameter in a `Vec`. Its
//! type arguments are walked like any type's; only a base type's argument is a marker.

use crate::abi::{FlatStr, Local};

pub struct Holder<'s, T> {
    pub stack: Local<'s>,
    pub heap: Vec<T>,
}

pub fn nested<'s>(h: Holder<'s, Local<'s>>) -> usize { // FLAG
    h.heap.len()
}

pub fn plain<'s>(h: Holder<'s, u32>) -> usize {
    h.heap.len()
}

pub struct Flats<'s> {
    pub list: Vec<Local<'s, FlatStr>>, // FLAG
}
