//! gc-lint case: values that stay where the GC sees them, or types that hold no value (exit 0).

use core::cell::Cell;
use core::marker::PhantomData;

use crate::abi::{Args, Cx, Local, PropertyDescriptor};

/// Builtin tables hold fn pointers.
pub type HostFn = for<'s> fn(&mut Cx<'s>, Local<'s>, Args<'s>, Local<'s>) -> Result<Local<'s>, ()>;
pub static TABLE: &[(&str, HostFn)] = &[];

/// Registered root ranges and VM data hold Values, not Locals.
pub struct Ranges {
    pub slots: Box<[Slot]>,
    pub values: Vec<Value>,
}

/// References, markers, fn pointers and trait objects hold no `Local`.
pub struct Views<'a, 's> {
    pub args: &'a [Local<'s>],
    pub refs: Vec<&'a Local<'s>>,
    pub marker: PhantomData<Local<'s>>,
    pub hooks: Vec<Box<dyn for<'x> Fn(Local<'x>) -> bool>>,
    pub ptrs: Vec<fn(Local<'static>)>,
}

thread_local! {
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

pub fn on_the_stack<'s>(l: Local<'s>, d: PropertyDescriptor<'s>) -> u64 {
    let pair = [l, l];
    let f = || pair[0].bits();
    let r = &l;
    let g = Box::new(move || r.bits());
    let n = 5u64;
    let h = Box::new(move || n);
    f() + g() + h() + d.value.map_or(0, |v| v.bits())
}
