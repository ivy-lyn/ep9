//! gc-lint case: a binding shadows a `Local` only where it is in scope: after its `let`, in
//! the `then` part of an `if let`, in the body of a `while let`.

use crate::abi::Local;

pub fn used_before_the_shadow<'s>(local: Local<'s>) -> Box<dyn Fn() -> (bool, i32) + 's> {
    Box::new(move || { // FLAG
        let answer = local.is_number();
        let local = 0;
        (answer, local)
    })
}

pub fn after_if_let<'s>(local: Local<'s>, o: Option<u32>) -> Box<dyn Fn() -> bool + 's> {
    if let Some(local) = o {
        assert!(local > 0);
    }
    Box::new(move || local.is_number()) // FLAG
}

pub fn after_while_let<'s>(local: Local<'s>, mut it: core::ops::Range<u32>) -> Box<dyn Fn() -> bool + 's> {
    while let Some(local) = it.next() {
        assert!(local > 0);
    }
    Box::new(move || local.is_number()) // FLAG
}

/// Shadows in scope at the use: no finding.
pub fn shadowed<'s>(local: Local<'s>, o: Option<u32>) -> Box<dyn Fn() -> u32 + 's> {
    assert!(local.is_number());
    if let Some(local) = o {
        return Box::new(move || local);
    }
    Box::new(move || {
        let local = 7;
        local
    })
}
