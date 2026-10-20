//! SP-Dispatch spike (M0 week 2, throwaway). See README.md.
//!
//! One opcode list (`ops.rs`) expands three ways (`dsl.rs`):
//! (a) a match loop, (b) sibling-call threaded code, (c) nightly `become`.

#![cfg_attr(feature = "become", feature(explicit_tail_calls))]
#![cfg_attr(feature = "become", allow(incomplete_features))]

pub mod asm;
pub mod dsl;
pub mod kernels;
pub mod ops;
pub mod program;
pub mod vm;

mod meta {
    #[allow(unused_imports)]
    use crate::dsl::{Flow, expand_meta};
    crate::ops::for_each_op!(expand_meta[]);
}

pub use meta::{OPS, Op, OpInfo};

const _: () = assert!(OPS.len() == 20, "the spike defines exactly 20 opcodes");
