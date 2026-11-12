//! The methods of `%TypedArray.prototype%`
//! (`sec-properties-of-the-%typedarray%-prototype-object`): module `typed_array_proto`, block
//! 36. Owner of this file: TypedArray TA-4 R1 (the one writer of the TA-4 lists; design
//! `docs/plan/notes/typedarray-design.md` TA-PROTO). The module joins the lazy group of
//! `array_buffer` (TA-D14): each table entry targets `%TypedArray.prototype%`.
//!
//! ```text
//! proto.rs       (R1) the ids, the properties (node v26's key order), MODULE
//! proto_iter.rs  (R3 PROTO-A) entries, keys, values, at, every, some, find, findIndex,
//!                     findLast, findLastIndex, forEach, includes, indexOf, lastIndexOf, join,
//!                     reduce, reduceRight, toLocaleString, toString, get @@toStringTag
//! proto_copy.rs  (R4 PROTO-B) copyWithin, fill, filter, map, reverse, set, slice, sort,
//!                     subarray, toReversed, toSorted, with
//! species.rs     (R4 PROTO-B) TypedArraySpeciesCreate, TypedArrayCreateSameType,
//!                     SpeciesConstructor
//! ```
//!
//! `[Symbol.iterator]` is the function of `values`, a `PropDef::alias`
//! (`sec-%typedarray%.prototype-%symbol.iterator%`: the same object). `toString` is a function
//! of its own with the steps of `Array.prototype.toString` (route TA-TOSTRING-IDENTITY:
//! `sec-%typedarray%.prototype.tostring` wants the same object).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-typed_array_proto.txt`. The ids follow node v26's key order. A missing
//! definition is appended at the end (36.32 and on), never inserted.

use ep9_vm::{
    BuiltinDef, BuiltinModule, IntrinsicId, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};

use super::{proto_copy, proto_iter};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 36;

/// The builtin functions, in id order (append-only). Lengths: the spec's (node v26 gives the same).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("entries", 0, proto_iter::entries), // 0
    BuiltinDef::method("keys", 0, proto_iter::keys),       // 1
    BuiltinDef::method("values", 0, proto_iter::values),   // 2
    BuiltinDef::method("at", 1, proto_iter::at),           // 3
    BuiltinDef::method("copyWithin", 2, proto_copy::copy_within), // 4
    BuiltinDef::method("every", 1, proto_iter::every),     // 5
    BuiltinDef::method("fill", 1, proto_copy::fill),       // 6
    BuiltinDef::method("filter", 1, proto_copy::filter),   // 7
    BuiltinDef::method("find", 1, proto_iter::find),       // 8
    BuiltinDef::method("findIndex", 1, proto_iter::find_index), // 9
    BuiltinDef::method("findLast", 1, proto_iter::find_last), // 10
    BuiltinDef::method("findLastIndex", 1, proto_iter::find_last_index), // 11
    BuiltinDef::method("forEach", 1, proto_iter::for_each), // 12
    BuiltinDef::method("includes", 1, proto_iter::includes), // 13
    BuiltinDef::method("indexOf", 1, proto_iter::index_of), // 14
    BuiltinDef::method("join", 1, proto_iter::join),       // 15
    BuiltinDef::method("lastIndexOf", 1, proto_iter::last_index_of), // 16
    BuiltinDef::method("map", 1, proto_copy::map),         // 17
    BuiltinDef::method("reverse", 0, proto_copy::reverse), // 18
    BuiltinDef::method("reduce", 1, proto_iter::reduce),   // 19
    BuiltinDef::method("reduceRight", 1, proto_iter::reduce_right), // 20
    BuiltinDef::method("set", 1, proto_copy::set),         // 21
    BuiltinDef::method("slice", 2, proto_copy::slice),     // 22
    BuiltinDef::method("some", 1, proto_iter::some),       // 23
    BuiltinDef::method("sort", 1, proto_copy::sort),       // 24
    BuiltinDef::method("subarray", 2, proto_copy::subarray), // 25
    BuiltinDef::method("toReversed", 0, proto_copy::to_reversed), // 26
    BuiltinDef::method("toSorted", 1, proto_copy::to_sorted), // 27
    BuiltinDef::method("with", 2, proto_copy::with),       // 28
    BuiltinDef::method("toLocaleString", 0, proto_iter::to_locale_string), // 29
    BuiltinDef::method("toString", 0, proto_iter::to_string), // 30
    BuiltinDef::getter("[Symbol.toStringTag]", proto_iter::to_string_tag), // 31
];

/// `key` = the function of definition `d` ({w, c}).
const fn method(key: &'static str, d: u16) -> PropDef {
    PropDef::method(PropKey::Str(key), d)
}

/// The properties this module adds: the methods of `%TypedArray.prototype%` after the
/// `constructor` and the 4 getters of module `typed_array` (TA-3), in node v26's key order
/// (`tests/typed_array.rs` `PROTOTYPE_KEYS`): the 31 methods ({w, c}), get `@@toStringTag`
/// ({c}) and `@@iterator` = `values` ({w, c}).
pub const PROPS: &[ObjectProps] = &[ObjectProps {
    target: IntrinsicId::TypedArrayPrototype,
    props: &[
        method("entries", 0),
        method("keys", 1),
        method("values", 2),
        method("at", 3),
        method("copyWithin", 4),
        method("every", 5),
        method("fill", 6),
        method("filter", 7),
        method("find", 8),
        method("findIndex", 9),
        method("findLast", 10),
        method("findLastIndex", 11),
        method("forEach", 12),
        method("includes", 13),
        method("indexOf", 14),
        method("join", 15),
        method("lastIndexOf", 16),
        method("map", 17),
        method("reverse", 18),
        method("reduce", 19),
        method("reduceRight", 20),
        method("set", 21),
        method("slice", 22),
        method("some", 23),
        method("sort", 24),
        method("subarray", 25),
        method("toReversed", 26),
        method("toSorted", 27),
        method("with", 28),
        method("toLocaleString", 29),
        method("toString", 30),
        PropDef::accessor(
            PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
            Some(31),
            None,
        ),
        PropDef::alias(
            PropKey::Symbol(WellKnownSymbol::ITERATOR),
            PropKey::Str("values"),
            PropAttrs::METHOD,
        ),
    ],
}];

/// The module ([`crate::TABLE`] installs it after `typed_array`).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "typed_array_proto",
    block: BLOCK,
    defs: DEFS,
    intrinsics: &[],
    props: PROPS,
};
