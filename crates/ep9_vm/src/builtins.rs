//! Builtin tables (H20): the definitions an embedder installs into a realm, and the ids of
//! the realm's intrinsics. Week-4 stub; P2 builds the tables.
#![allow(dead_code)] // wk4-stub

use crate::runtime::{Root, Runtime};

/// A table of builtin definitions (`IsolateConfig::builtins`); `ep9_builtins::TABLE` is the
/// engine's. Empty until the table types exist.
#[derive(Debug)]
pub struct BuiltinTable {}

/// Per-isolate builtin state (`Runtime::builtins`).
#[derive(Debug, Default)]
pub(crate) struct BuiltinState {}

/// The intrinsic objects of a realm that builtins name (§6.1.7.4), with stable ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
#[allow(clippy::enum_variant_names)] // wk4-stub: only prototypes so far
pub enum IntrinsicId {
    ObjectPrototype,
    FunctionPrototype,
    ArrayPrototype,
    StringPrototype,
    NumberPrototype,
    BooleanPrototype,
    ErrorPrototype,
    TypeErrorPrototype,
    RangeErrorPrototype,
    ReferenceErrorPrototype,
    SyntaxErrorPrototype,
    EvalErrorPrototype,
    UriErrorPrototype,
    ArrayIteratorPrototype,
}

impl IntrinsicId {
    pub const ALL: [IntrinsicId; 14] = [
        IntrinsicId::ObjectPrototype,
        IntrinsicId::FunctionPrototype,
        IntrinsicId::ArrayPrototype,
        IntrinsicId::StringPrototype,
        IntrinsicId::NumberPrototype,
        IntrinsicId::BooleanPrototype,
        IntrinsicId::ErrorPrototype,
        IntrinsicId::TypeErrorPrototype,
        IntrinsicId::RangeErrorPrototype,
        IntrinsicId::ReferenceErrorPrototype,
        IntrinsicId::SyntaxErrorPrototype,
        IntrinsicId::EvalErrorPrototype,
        IntrinsicId::UriErrorPrototype,
        IntrinsicId::ArrayIteratorPrototype,
    ];

    /// The runtime root that holds this intrinsic.
    pub(crate) const fn root(self) -> Root {
        match self {
            IntrinsicId::ObjectPrototype => Root::ObjectPrototype,
            IntrinsicId::FunctionPrototype => Root::FunctionPrototype,
            IntrinsicId::ArrayPrototype => Root::ArrayPrototype,
            IntrinsicId::StringPrototype => Root::StringPrototype,
            IntrinsicId::NumberPrototype => Root::NumberPrototype,
            IntrinsicId::BooleanPrototype => Root::BooleanPrototype,
            IntrinsicId::ErrorPrototype => Root::ErrorPrototype,
            IntrinsicId::TypeErrorPrototype => Root::TypeErrorPrototype,
            IntrinsicId::RangeErrorPrototype => Root::RangeErrorPrototype,
            IntrinsicId::ReferenceErrorPrototype => Root::ReferenceErrorPrototype,
            IntrinsicId::SyntaxErrorPrototype => Root::SyntaxErrorPrototype,
            IntrinsicId::EvalErrorPrototype => Root::EvalErrorPrototype,
            IntrinsicId::UriErrorPrototype => Root::UriErrorPrototype,
            IntrinsicId::ArrayIteratorPrototype => Root::ArrayIteratorPrototype,
        }
    }
}

/// Installs the embedder's table into the realm (last step of `realm::init`).
pub(crate) fn install(_rt: &Runtime, _table: Option<&'static BuiltinTable>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intrinsic_ids_are_dense_and_map_to_distinct_roots() {
        let mut roots = Vec::new();
        for (i, id) in IntrinsicId::ALL.iter().enumerate() {
            assert_eq!(*id as usize, i);
            roots.push(id.root() as u8);
        }
        roots.sort_unstable();
        roots.dedup();
        assert_eq!(roots.len(), IntrinsicId::ALL.len());
    }
}
