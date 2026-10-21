//! gc-lint case: a closure that captures a `Local`, given to `Box::new`. The type of `first`
//! comes from the return type of `Args::get`.

use crate::abi::{Args, Cx, Local};

impl<'s> Args<'s> {
    pub fn get(&self, i: usize) -> Local<'s> {
        todo!("{i}")
    }
}

pub fn later<'s>(_cx: &mut Cx<'s>, args: Args<'s>) -> Box<dyn Fn() -> u64 + 's> {
    let first = args.get(0);
    Box::new(move || first.bits()) // FLAG
}
