//! gc-lint case: an allowed finding. Exit 0, and the allow is listed.

use crate::abi::Local;

// gc-lint: allow(fixture: a reviewed exception)
pub fn keep<'s>(v: Vec<Local<'s>>) -> usize {
    v.len()
}
