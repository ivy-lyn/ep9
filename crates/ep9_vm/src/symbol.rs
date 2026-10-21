//! Symbols (§6.1.5.1): the SYMBOL cell, the well-known symbols and the `Symbol.for`
//! registry. Week-4 stub; P2 builds them.
#![allow(dead_code)] // wk4-stub

use ep9_heap::Heap;

use crate::frame::UNDEFINED;
use crate::runtime::Runtime;

/// Per-isolate symbol state (`Runtime::symbols`): the registry and the identity-hash PRNG.
#[derive(Debug, Default)]
pub(crate) struct SymbolState {}

/// The well-known symbols (§6.1.5.1 table 1), plus `dispose` and `asyncDispose` (explicit
/// resource management). Stable order: the isolate roots follow it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum WellKnownSymbol {
    AsyncIterator,
    HasInstance,
    IsConcatSpreadable,
    Iterator,
    Match,
    MatchAll,
    Replace,
    Search,
    Species,
    Split,
    ToPrimitive,
    ToStringTag,
    Unscopables,
    Dispose,
    AsyncDispose,
}

impl WellKnownSymbol {
    pub(crate) const ALL: [WellKnownSymbol; 15] = [
        WellKnownSymbol::AsyncIterator,
        WellKnownSymbol::HasInstance,
        WellKnownSymbol::IsConcatSpreadable,
        WellKnownSymbol::Iterator,
        WellKnownSymbol::Match,
        WellKnownSymbol::MatchAll,
        WellKnownSymbol::Replace,
        WellKnownSymbol::Search,
        WellKnownSymbol::Species,
        WellKnownSymbol::Split,
        WellKnownSymbol::ToPrimitive,
        WellKnownSymbol::ToStringTag,
        WellKnownSymbol::Unscopables,
        WellKnownSymbol::Dispose,
        WellKnownSymbol::AsyncDispose,
    ];

    /// The `Symbol` property name (`iterator` for `Symbol.iterator`).
    pub(crate) const fn name(self) -> &'static str {
        match self {
            WellKnownSymbol::AsyncIterator => "asyncIterator",
            WellKnownSymbol::HasInstance => "hasInstance",
            WellKnownSymbol::IsConcatSpreadable => "isConcatSpreadable",
            WellKnownSymbol::Iterator => "iterator",
            WellKnownSymbol::Match => "match",
            WellKnownSymbol::MatchAll => "matchAll",
            WellKnownSymbol::Replace => "replace",
            WellKnownSymbol::Search => "search",
            WellKnownSymbol::Species => "species",
            WellKnownSymbol::Split => "split",
            WellKnownSymbol::ToPrimitive => "toPrimitive",
            WellKnownSymbol::ToStringTag => "toStringTag",
            WellKnownSymbol::Unscopables => "unscopables",
            WellKnownSymbol::Dispose => "dispose",
            WellKnownSymbol::AsyncDispose => "asyncDispose",
        }
    }
}

/// Registers the SYMBOL cell kind.
pub(crate) fn register_kinds(_heap: &Heap) {}

/// Creates the well-known symbols and the registry. Runs in `realm::init`.
pub(crate) fn install(_rt: &Runtime) {}

/// The well-known symbol `w` of this isolate (undefined until symbols exist).
pub(crate) fn well_known(_rt: &Runtime, _w: WellKnownSymbol) -> u64 {
    UNDEFINED
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_order_and_names() {
        for (i, w) in WellKnownSymbol::ALL.iter().enumerate() {
            assert_eq!(*w as usize, i);
        }
        let mut names: Vec<&str> = WellKnownSymbol::ALL.iter().map(|w| w.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 15);
    }
}
