//! gc-lint case: `Box::new` and `Rc::new` called through a `use .. as` name.

use std::boxed::Box as HeapBox;
use std::rc::Rc as Shared;

use crate::abi::Local;

pub fn later<'s>(local: Local<'s>) -> HeapBox<dyn Fn() -> bool + 's> {
    HeapBox::new(move || local.is_number()) // FLAG
}

pub fn shared<'s>(local: Local<'s>) -> Shared<dyn Fn() -> u64 + 's> {
    let n = 5u64;
    let _plain = Shared::new(move || n);
    Shared::new(move || local.bits()) // FLAG
}
