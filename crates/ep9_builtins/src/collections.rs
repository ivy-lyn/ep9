//! Map, Set, WeakMap and WeakSet (§24.1-§24.4): module `collections`, block 13. Owner: M3-P1
//! B-COLLECTIONS. Makes `%Map%`, `%Set%`, `%WeakMap%`, `%WeakSet%`, their prototypes,
//! `%MapIteratorPrototype%` and `%SetIteratorPrototype%` (`IntrinsicId` 52-61).
//!
//! ```text
//! this file: the table, AddEntriesFromIterable, CreateIteratorResultObject
//!   collections/map.rs   Map, Map.groupBy, Map.prototype, %MapIteratorPrototype%.next,
//!                        the upsert methods (getOrInsert, getOrInsertComputed)
//!   collections/set.rs   Set, Set.prototype, %SetIteratorPrototype%.next, the Set methods
//!                        (union, intersection, ...; GetSetRecord)
//!   collections/weak.rs  WeakMap, WeakSet (CanBeHeldWeakly keys)
//! the storage: ep9_vm (`abi/collections.rs` over ORDERED_TABLE cells, `ordered_table.rs`):
//!   new_collection, collection_get/has/set/delete/clear/size, collection_cursor/next,
//!   new_collection_iterator, collection_iterator_next, can_be_held_weakly
//! ```
//!
//! A collection is a SLOT_OBJECT of the class Map, Set, WeakMap or WeakSet; its one internal
//! slot holds the table, which no builtin here sees. Keys compare by SameValueZero (-0 is
//! stored as +0). Iteration walks the entries in insertion order, sees the entries added
//! during the walk and skips the deleted ones.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-collections.txt`.

mod map;
mod set;
mod weak;

use ep9_vm::abi::{IteratorRecord, JsObject};
use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectProps, PropAttrs, PropDef, PropKey, WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 13;

mod def {
    pub const MAP: u16 = 0;
    pub const MAP_GROUP_BY: u16 = 1;
    pub const MAP_SPECIES: u16 = 2;
    pub const MAP_GET: u16 = 3;
    pub const MAP_SET: u16 = 4;
    pub const MAP_HAS: u16 = 5;
    pub const MAP_DELETE: u16 = 6;
    pub const MAP_CLEAR: u16 = 7;
    pub const MAP_ENTRIES: u16 = 8;
    pub const MAP_FOR_EACH: u16 = 9;
    pub const MAP_KEYS: u16 = 10;
    pub const MAP_SIZE: u16 = 11;
    pub const MAP_VALUES: u16 = 12;
    pub const MAP_ITERATOR_NEXT: u16 = 13;
    pub const SET: u16 = 14;
    pub const SET_SPECIES: u16 = 15;
    pub const SET_HAS: u16 = 16;
    pub const SET_ADD: u16 = 17;
    pub const SET_DELETE: u16 = 18;
    pub const SET_DIFFERENCE: u16 = 19;
    pub const SET_CLEAR: u16 = 20;
    pub const SET_ENTRIES: u16 = 21;
    pub const SET_FOR_EACH: u16 = 22;
    pub const SET_INTERSECTION: u16 = 23;
    pub const SET_IS_SUBSET_OF: u16 = 24;
    pub const SET_IS_SUPERSET_OF: u16 = 25;
    pub const SET_IS_DISJOINT_FROM: u16 = 26;
    pub const SET_SIZE: u16 = 27;
    pub const SET_SYMMETRIC_DIFFERENCE: u16 = 28;
    pub const SET_UNION: u16 = 29;
    pub const SET_VALUES: u16 = 30;
    pub const SET_ITERATOR_NEXT: u16 = 31;
    pub const WEAK_MAP: u16 = 32;
    pub const WEAK_MAP_DELETE: u16 = 33;
    pub const WEAK_MAP_GET: u16 = 34;
    pub const WEAK_MAP_SET: u16 = 35;
    pub const WEAK_MAP_HAS: u16 = 36;
    pub const WEAK_SET: u16 = 37;
    pub const WEAK_SET_DELETE: u16 = 38;
    pub const WEAK_SET_HAS: u16 = 39;
    pub const WEAK_SET_ADD: u16 = 40;
    pub const MAP_GET_OR_INSERT: u16 = 41;
    pub const MAP_GET_OR_INSERT_COMPUTED: u16 = 42;
    pub const WEAK_MAP_GET_OR_INSERT: u16 = 43;
    pub const WEAK_MAP_GET_OR_INSERT_COMPUTED: u16 = 44;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Map", 0, map::map_ctor), // 0
    BuiltinDef::method("groupBy", 2, map::group_by),  // 1
    BuiltinDef::getter("[Symbol.species]", species),  // 2
    BuiltinDef::method("get", 1, map::get),           // 3
    BuiltinDef::method("set", 2, map::set),           // 4
    BuiltinDef::method("has", 1, map::has),           // 5
    BuiltinDef::method("delete", 1, map::delete),     // 6
    BuiltinDef::method("clear", 0, map::clear),       // 7
    BuiltinDef::method("entries", 0, map::entries),   // 8
    BuiltinDef::method("forEach", 1, map::for_each),  // 9
    BuiltinDef::method("keys", 0, map::keys),         // 10
    BuiltinDef::getter("size", map::size),            // 11
    BuiltinDef::method("values", 0, map::values),     // 12
    BuiltinDef::method("next", 0, map::iterator_next), // 13
    BuiltinDef::constructor("Set", 0, set::set_ctor), // 14
    BuiltinDef::getter("[Symbol.species]", species),  // 15
    BuiltinDef::method("has", 1, set::has),           // 16
    BuiltinDef::method("add", 1, set::add),           // 17
    BuiltinDef::method("delete", 1, set::delete),     // 18
    BuiltinDef::method("difference", 1, set::difference), // 19
    BuiltinDef::method("clear", 0, set::clear),       // 20
    BuiltinDef::method("entries", 0, set::entries),   // 21
    BuiltinDef::method("forEach", 1, set::for_each),  // 22
    BuiltinDef::method("intersection", 1, set::intersection), // 23
    BuiltinDef::method("isSubsetOf", 1, set::is_subset_of), // 24
    BuiltinDef::method("isSupersetOf", 1, set::is_superset_of), // 25
    BuiltinDef::method("isDisjointFrom", 1, set::is_disjoint_from), // 26
    BuiltinDef::getter("size", set::size),            // 27
    BuiltinDef::method("symmetricDifference", 1, set::symmetric_difference), // 28
    BuiltinDef::method("union", 1, set::union),       // 29
    BuiltinDef::method("values", 0, set::values),     // 30
    BuiltinDef::method("next", 0, set::iterator_next), // 31
    BuiltinDef::constructor("WeakMap", 0, weak::weak_map_ctor), // 32
    BuiltinDef::method("delete", 1, weak::weak_map_delete), // 33
    BuiltinDef::method("get", 1, weak::weak_map_get), // 34
    BuiltinDef::method("set", 2, weak::weak_map_set), // 35
    BuiltinDef::method("has", 1, weak::weak_map_has), // 36
    BuiltinDef::constructor("WeakSet", 0, weak::weak_set_ctor), // 37
    BuiltinDef::method("delete", 1, weak::weak_set_delete), // 38
    BuiltinDef::method("has", 1, weak::weak_set_has), // 39
    BuiltinDef::method("add", 1, weak::weak_set_add), // 40
    BuiltinDef::method("getOrInsert", 2, map::get_or_insert), // 41
    BuiltinDef::method("getOrInsertComputed", 2, map::get_or_insert_computed), // 42
    BuiltinDef::method("getOrInsert", 2, weak::weak_map_get_or_insert), // 43
    BuiltinDef::method(
        "getOrInsertComputed",
        2,
        weak::weak_map_get_or_insert_computed,
    ), // 44
];

const fn object(id: IntrinsicId, proto: IntrinsicId) -> IntrinsicDef {
    IntrinsicDef {
        id,
        value: IntrinsicValue::Object { proto: Some(proto) },
    }
}

const fn builtin(id: IntrinsicId, d: u16) -> IntrinsicDef {
    IntrinsicDef {
        id,
        value: IntrinsicValue::Builtin(d),
    }
}

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    object(IntrinsicId::MapPrototype, IntrinsicId::ObjectPrototype),
    builtin(IntrinsicId::Map, def::MAP),
    object(IntrinsicId::SetPrototype, IntrinsicId::ObjectPrototype),
    builtin(IntrinsicId::Set, def::SET),
    object(IntrinsicId::WeakMapPrototype, IntrinsicId::ObjectPrototype),
    builtin(IntrinsicId::WeakMap, def::WEAK_MAP),
    object(IntrinsicId::WeakSetPrototype, IntrinsicId::ObjectPrototype),
    builtin(IntrinsicId::WeakSet, def::WEAK_SET),
    object(
        IntrinsicId::MapIteratorPrototype,
        IntrinsicId::IteratorPrototype,
    ),
    object(
        IntrinsicId::SetIteratorPrototype,
        IntrinsicId::IteratorPrototype,
    ),
];

const fn method(name: &'static str, d: u16) -> PropDef {
    PropDef::method(PropKey::Str(name), d)
}

/// `constructor` = `id` ({w, c}).
const fn constructor(id: IntrinsicId) -> PropDef {
    PropDef::intrinsic(PropKey::Atom(Atom::Constructor), id, PropAttrs::METHOD)
}

/// `prototype` = `id` ({}).
const fn prototype(id: IntrinsicId) -> PropDef {
    PropDef::intrinsic(PropKey::Atom(Atom::Prototype), id, PropAttrs::NONE)
}

/// `@@toStringTag` = `tag` ({c}).
const fn to_string_tag(tag: &'static str) -> PropDef {
    PropDef::constant(
        PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
        ConstValue::Str(tag),
        PropAttrs::CONFIGURABLE,
    )
}

/// `@@iterator` = the function of `of` on the same object ({w, c}).
const fn iterator_alias(of: &'static str) -> PropDef {
    PropDef::alias(
        PropKey::Symbol(WellKnownSymbol::ITERATOR),
        PropKey::Str(of),
        PropAttrs::METHOD,
    )
}

/// `get [ @@species ]` ({c}).
const fn species_accessor(d: u16) -> PropDef {
    PropDef::accessor(PropKey::Symbol(WellKnownSymbol::SPECIES), Some(d), None)
}

/// The properties this module adds, per object (the order of node v24's own keys; the
/// upsert methods, which node v24 does not have, after `values` and `set`).
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Map,
        props: &[
            prototype(IntrinsicId::MapPrototype),
            method("groupBy", def::MAP_GROUP_BY),
            species_accessor(def::MAP_SPECIES),
        ],
    },
    ObjectProps {
        target: IntrinsicId::MapPrototype,
        props: &[
            constructor(IntrinsicId::Map),
            method("get", def::MAP_GET),
            method("set", def::MAP_SET),
            method("has", def::MAP_HAS),
            method("delete", def::MAP_DELETE),
            method("clear", def::MAP_CLEAR),
            method("entries", def::MAP_ENTRIES),
            method("forEach", def::MAP_FOR_EACH),
            method("keys", def::MAP_KEYS),
            PropDef::accessor(PropKey::Str("size"), Some(def::MAP_SIZE), None),
            method("values", def::MAP_VALUES),
            method("getOrInsert", def::MAP_GET_OR_INSERT),
            method("getOrInsertComputed", def::MAP_GET_OR_INSERT_COMPUTED),
            to_string_tag("Map"),
            iterator_alias("entries"),
        ],
    },
    ObjectProps {
        target: IntrinsicId::MapIteratorPrototype,
        props: &[
            method("next", def::MAP_ITERATOR_NEXT),
            to_string_tag("Map Iterator"),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Set,
        props: &[
            prototype(IntrinsicId::SetPrototype),
            species_accessor(def::SET_SPECIES),
        ],
    },
    ObjectProps {
        target: IntrinsicId::SetPrototype,
        props: &[
            constructor(IntrinsicId::Set),
            method("has", def::SET_HAS),
            method("add", def::SET_ADD),
            method("delete", def::SET_DELETE),
            method("difference", def::SET_DIFFERENCE),
            method("clear", def::SET_CLEAR),
            method("entries", def::SET_ENTRIES),
            method("forEach", def::SET_FOR_EACH),
            method("intersection", def::SET_INTERSECTION),
            method("isSubsetOf", def::SET_IS_SUBSET_OF),
            method("isSupersetOf", def::SET_IS_SUPERSET_OF),
            method("isDisjointFrom", def::SET_IS_DISJOINT_FROM),
            PropDef::accessor(PropKey::Str("size"), Some(def::SET_SIZE), None),
            method("symmetricDifference", def::SET_SYMMETRIC_DIFFERENCE),
            method("union", def::SET_UNION),
            method("values", def::SET_VALUES),
            // §24.2.4.12 Set.prototype.keys is %Set.prototype.values%.
            PropDef::alias(
                PropKey::Str("keys"),
                PropKey::Str("values"),
                PropAttrs::METHOD,
            ),
            to_string_tag("Set"),
            iterator_alias("values"),
        ],
    },
    ObjectProps {
        target: IntrinsicId::SetIteratorPrototype,
        props: &[
            method("next", def::SET_ITERATOR_NEXT),
            to_string_tag("Set Iterator"),
        ],
    },
    ObjectProps {
        target: IntrinsicId::WeakMap,
        props: &[prototype(IntrinsicId::WeakMapPrototype)],
    },
    ObjectProps {
        target: IntrinsicId::WeakMapPrototype,
        props: &[
            constructor(IntrinsicId::WeakMap),
            method("delete", def::WEAK_MAP_DELETE),
            method("get", def::WEAK_MAP_GET),
            method("set", def::WEAK_MAP_SET),
            method("has", def::WEAK_MAP_HAS),
            method("getOrInsert", def::WEAK_MAP_GET_OR_INSERT),
            method("getOrInsertComputed", def::WEAK_MAP_GET_OR_INSERT_COMPUTED),
            to_string_tag("WeakMap"),
        ],
    },
    ObjectProps {
        target: IntrinsicId::WeakSet,
        props: &[prototype(IntrinsicId::WeakSetPrototype)],
    },
    ObjectProps {
        target: IntrinsicId::WeakSetPrototype,
        props: &[
            constructor(IntrinsicId::WeakSet),
            method("delete", def::WEAK_SET_DELETE),
            method("has", def::WEAK_SET_HAS),
            method("add", def::WEAK_SET_ADD),
            to_string_tag("WeakSet"),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.3 Map, Set, WeakMap, WeakSet ( . . . ) ({w, c}).
        props: &[
            PropDef::intrinsic(PropKey::Str("Map"), IntrinsicId::Map, PropAttrs::METHOD),
            PropDef::intrinsic(PropKey::Str("Set"), IntrinsicId::Set, PropAttrs::METHOD),
            PropDef::intrinsic(
                PropKey::Str("WeakMap"),
                IntrinsicId::WeakMap,
                PropAttrs::METHOD,
            ),
            PropDef::intrinsic(
                PropKey::Str("WeakSet"),
                IntrinsicId::WeakSet,
                PropAttrs::METHOD,
            ),
        ],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "collections",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- shared steps --------------------------------------------------------------------------

/// get Map [ @@species ] (§24.1.2.3) and get Set [ @@species ] (§24.2.3.2): the this value.
fn species<'s>(
    _cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(this)
}

/// The TypeError of a constructor called without `new` (§24.1.1.1 step 1 and the others).
fn requires_new<'s>(cx: &mut Cx<'s>, name: &str) -> ep9_vm::Throw {
    let msg = format!("Constructor {name} requires 'new'");
    cx.throw_type_error(&msg)
}

/// CreateIteratorResultObject(value, done) (§7.4.14).
fn iter_result<'s>(cx: &mut Cx<'s>, value: Local<'s>, done: bool) -> JsResult<Local<'s>> {
    let o = cx.new_object();
    cx.create_data_property_or_throw(o, "value", value)?;
    cx.create_data_property_or_throw(o, "done", Local::boolean(done))?;
    Ok(o.value())
}

/// How AddEntriesFromIterable (§24.1.1.2) calls the adder: with the two entry values (Map,
/// WeakMap), or the Set constructors' step 8 loop with the value itself (Set, WeakSet).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Adder {
    Entries,
    Values,
}

/// The constructor steps after the collection exists (§24.1.1.1 steps 4-7, §24.2.2.1 steps
/// 4-8 and the weak ones): nothing for an undefined or null `iterable`; else `adder` =
/// ? Get(target, `adder_name`), which must be callable, and every value of the iterable goes
/// to it (AddEntriesFromIterable for an entry adder). An abrupt completion of a step closes
/// the iterator.
fn add_from_iterable<'s>(
    cx: &mut Cx<'s>,
    target: Local<'s, JsObject>,
    iterable: Local<'s>,
    adder_name: &str,
    how: Adder,
) -> JsResult<()> {
    if iterable.is_undefined_or_null() {
        return Ok(());
    }
    let adder = cx.get(target, adder_name)?;
    if !cx.is_callable(adder) {
        let msg = format!(
            "'{adder_name}' returned for property '{adder_name}' of object is not a function"
        );
        return Err(cx.throw_type_error(&msg));
    }
    let mut record = cx.get_iterator(iterable)?;
    while let Some(next) = cx.iterator_step_value(&mut record)? {
        if let Err(throw) = add_one(cx, target, adder, next, how) {
            return cx.iterator_close(&record, Err(throw));
        }
    }
    Ok(())
}

/// One value of [`add_from_iterable`]: Call(adder, target, « next ») for a value adder; for
/// an entry adder, AddEntriesFromIterable steps 2.c-2.i (the value must be an object; its
/// "0" and "1" go to the adder).
fn add_one<'s>(
    cx: &mut Cx<'s>,
    target: Local<'s, JsObject>,
    adder: Local<'s>,
    next: Local<'s>,
    how: Adder,
) -> JsResult<()> {
    if how == Adder::Values {
        cx.call(adder, target, &[next])?;
        return Ok(());
    }
    // c. If next is not an Object, then let error be ThrowCompletion(a TypeError).
    let Some(entry) = next.as_object() else {
        return Err(cx.throw_type_error("Iterator value is not an entry object"));
    };
    // d. Let k be Completion(Get(next, "0")).
    let k = cx.get_index(entry, 0)?;
    // f. Let v be Completion(Get(next, "1")).
    let v = cx.get_index(entry, 1)?;
    // h. Let status be Completion(Call(adder, target, « k, v »)).
    cx.call(adder, target, &[k, v])?;
    Ok(())
}

/// Closes `record` after a normal completion and gives `result` (IteratorClose(record,
/// NormalCompletion(unused)), then return `result`).
fn close_with<'s>(
    cx: &mut Cx<'s>,
    record: &IteratorRecord<'s>,
    result: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.iterator_close(record, Ok(result))
}

/// 𝔽(n).
fn number<'s>(n: usize) -> Local<'s> {
    Local::number(n as f64)
}
