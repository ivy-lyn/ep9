//! gc-lint case (with `store.rs`): std's `Layout` under engine type names, in another file.

use std::alloc::Layout as Held;
use std::alloc::Layout as Local;

pub fn size(l: Local, h: Held) -> usize {
    l.size() + h.size()
}
