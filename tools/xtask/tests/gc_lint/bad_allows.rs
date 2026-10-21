//! gc-lint case: wrong allows are findings (exit 1): the allow on line 6 covers no finding,
//! the one on line 9 is not followed by an item, lines 12 and 13 are malformed.

use crate::abi::Local;

// gc-lint: allow(nothing here to allow)
pub fn plain() {}

// gc-lint: allow(a blank line follows)

pub fn other() {}
// gc-lint: allow()
// gc-lint: alow(a typo)
pub fn third(_: &[Local<'_>]) {}
