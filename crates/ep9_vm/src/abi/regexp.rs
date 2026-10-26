#![allow(dead_code)] // m4-stub
//! RegExp on the builtin ABI (owners M4-P2 RX0A, then M4-P3 REGEXP1; V9-D15): the part-2 `Cx`
//! methods through which `ep9_builtins` reaches `ep9_regexp` (a dependency of `ep9_vm` only,
//! dep-guards check 8), and the match record they return.
//!
//! ```text
//! ep9_builtins regexp.rs ──Cx::regexp_create / Cx::regexp_exec (RX0A)──> regexp.rs (VM)
//!                                                                         └─> ep9_regexp
//! ```
//!
//! M4-P0 declares the module, its `impl Cx` block (empty) and the [`MatchRecord`] type, because
//! `abi/mod.rs` (its `mod` and `pub use` lines) is frozen after M4-P0. A new method is a part-2
//! addition: a new `h20-core.txt` line (EP9_BLESS) and an H20 Change-log row (V9-A1, V9-A14).

use super::Cx;

/// The result of one successful match: the capture spans in UTF-16 code units (index 0 is the
/// whole match; `None` for a capture that did not take part). Rust memory with no Value in it
/// (plan rule 8), so it may outlive a GC point. RX0A gives it its methods.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MatchRecord {
    captures: Vec<Option<(usize, usize)>>,
}

impl<'s> Cx<'s> {}
