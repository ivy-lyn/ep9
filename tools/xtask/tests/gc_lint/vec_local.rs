//! gc-lint case: `Vec<Local>`, a list of values that the GC cannot see.

use crate::abi::{Args, Cx, Local};

pub struct Saved<'s> {
    pub list: Vec<Local<'s>>, // FLAG
}

pub fn collect_args<'s>(_cx: &mut Cx<'s>, args: Args<'s>) -> usize {
    let mut out: Vec<Local<'s>> = Vec::with_capacity(args.len()); // FLAG
    for a in args.iter() {
        out.push(a);
    }
    out.len()
}
