#![allow(dead_code)] // m4-stub
//! The Spike C IR (stub; SPIKEC-1 builds it in M4-P1): an SSA graph of one function, with a
//! fact on each value and the H30 rules.
//!
//! ```text
//!  unknown                         generic path, no guard
//!     |
//!  predicted{source}               RUNTIME | PROFILED | HINTED (FactSource); needs a Guard (H32)
//!     |
//!  proven                          no guard: `&`, `|`, `^`, `~`, `<<`, `>>` give int32 (H30 PROVEN row)
//! ```
//!
//! | H30 rule | What this IR does (SPIKEC-1) |
//! |---|---|
//! | proven ⊑ predicted{source} ⊑ unknown | each value carries a [`Trust`]; a join never moves a value down |
//! | no raw heap constant; only the five ref nodes | a heap constant is a [`RefNode`]; there is no other way to name a cell |
//! | the guard policy is chosen per mode in the IR (H32) | a predicted fact gets a guard node whose failure, in JIT mode, is an eager deopt exit (until month 5: the "deopt not built" trap) |
//! | one graph builder for live and PROFILED facts | [`crate::build`] reads a FactSnapshot in both paths |

use ep9_facts::FactSource;

/// A value or node of a [`Graph`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u32);

/// The trust level of a fact (H30). The order of the variants is the lattice order:
/// `Proven` ⊑ `Predicted` ⊑ `Unknown`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// Holds on every path: no guard.
    Proven,
    /// Seen by a fact source; holds only behind a guard (H32).
    Predicted(FactSource),
    /// Nothing known: the generic path.
    Unknown,
}

/// The five ref nodes of H30: the only way the IR names a heap value (no raw heap constant).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RefNode {
    /// An isolate root by stable index (H13).
    RootRef(u32),
    /// A realm intrinsic by stable index (`IntrinsicId`).
    IntrinsicRef(u32),
    /// A shape used in a guard.
    ShapeRef(u32),
    /// A RW IC or link cell.
    CellRef(u32),
    /// A constant of the unit's constant pool.
    UnitConst(u32),
}

/// The SSA graph of one function (SPIKEC-1).
#[derive(Debug, Default)]
pub struct Graph {
    _built_by_spikec_1: (),
}
