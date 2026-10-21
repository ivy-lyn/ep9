//! gc-lint case: a `thread_local!` that holds a `Local`.

use core::cell::Cell;

use crate::abi::Local;

thread_local! {
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    static LAST: Cell<Option<Local<'static>>> = const { Cell::new(None) }; // FLAG
}
