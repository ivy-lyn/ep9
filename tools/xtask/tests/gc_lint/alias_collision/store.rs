//! gc-lint case: a `use .. as` in another file never hides a stack-only type. Here `Held` is
//! the engine's `Local`; `unrelated.rs` names std's `Layout` both `Local` and `Held`.

use core::marker::PhantomData as Marker;
use std::collections::VecDeque as Queue;

use crate::abi::Local as Held;

pub struct Saved<'s> {
    pub qualified: Vec<crate::abi::Local<'s>>, // FLAG
    pub held: Vec<Held<'s>>, // FLAG
    pub queued: Queue<Held<'s>>, // FLAG
    pub marker: Marker<Held<'s>>,
}
