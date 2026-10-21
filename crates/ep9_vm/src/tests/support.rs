//! Host functions and helpers for the VM tests (`print` and its output buffer).

use std::cell::RefCell;

use crate::{Args, Cx, JsResult, Local};

thread_local! {
    static OUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// `print(...args)`: ToString of each argument, joined by spaces, into the test output.
pub(crate) fn print<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let mut parts = Vec::new();
    for v in args.iter() {
        parts.push(cx.to_rust_string(v)?);
    }
    OUT.with(|o| o.borrow_mut().push(parts.join(" ")));
    Ok(Local::undefined())
}

/// The lines `print` wrote since the last call.
pub(crate) fn take_output() -> Vec<String> {
    OUT.with(|o| core::mem::take(&mut *o.borrow_mut()))
}
