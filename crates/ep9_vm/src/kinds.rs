//! Cell kind ids (word0 byte 5), table v1 (v2 of the object layout adds GENERATOR and
//! SLOT_OBJECT, M2-P0; v3 adds ARGUMENTS, MODULE_NAMESPACE, MODULE, ORDERED_TABLE and REALM,
//! M3-P0; v4 adds PROXY, PROXY_CALLABLE, VALIDITY_CELL and WEAK_CELL and the CONS, SLICED and
//! THIN layouts, M4-P0), and the kind tests on Value bits.
//!
//! | Ids | Kinds | Range test |
//! |---|---|---|
//! | 1-7 | strings: SEQ_LATIN1, SEQ_UTF16; CONS, SLICED, THIN (layouts below, OBJECT_LAYOUT 4; built by M4-P1 STR1 and M4-P2 STR2); EXT_LATIN1, EXT_UTF16 reserved | [`is_string`] |
//! | 8 | SYMBOL | [`is_symbol`] |
//! | 16-63 | JS objects: OBJECT, FUNCTION, ARRAY, ERROR, ITERATOR, PRIMITIVE_WRAPPER, BOUND_FUNCTION, GENERATOR, SLOT_OBJECT, ARGUMENTS, MODULE_NAMESPACE, PROXY, PROXY_CALLABLE; 29-63 reserved | [`is_object`] |
//! | 64-127 | internal cells: PROPS, ELEMENTS, CONTEXT, ACCESSOR_PAIR, FEEDBACK, POLY_IC, IC_HANDLER, SLOTS, DESCRIPTORS, DESC_INDEX, TRANSITIONS, PROTO_INFO, MODULE, ORDERED_TABLE, REALM, VALIDITY_CELL, WEAK_CELL | - |
//! | 128 | SHAPE | - |
//!
//! OBJECT_LAYOUT 4 (M4-P0; plan v9 V9-D19, V9-D23, V9-A8). The ids below are the whole
//! month-4 list: no other kind id comes in month 4 (construct sites and instanceof caches are
//! IC_HANDLER cells with new handler kinds, `ic.rs`; a literal site needs no cell). P0 declares
//! the ids and layouts and registers no kind: the role that first allocates a kind registers
//! its trace function.
//!
//! | Kind | Id | Layout (byte offsets) | Builder |
//! |---|---|---|---|
//! | CONS | 3 | the string header (`string.rs`: +8 length (u32), +12 hash field (u32)), +16 first, +24 second (strings) | M4-P1 STR1 |
//! | SLICED | 4 | the string header (+8 length (u32), +12 hash field (u32)), +16 parent (a flat string), +24 offset (int32) | M4-P1 STR1 |
//! | THIN | 5 | the string header (+8 length (u32), +12 hash field (u32)), +16 actual (the atom it forwards to) | M4-P2 STR2 |
//! | PROXY | 27 | a Proxy exotic object (§10.5) whose target is not callable: `[[ProxyTarget]]` and `[[ProxyHandler]]` in fixed fields (PROXY1 sets the offsets) | M4-P4 PROXY1 (`proxy.rs`) |
//! | PROXY_CALLABLE | 28 | a Proxy whose target is callable (IsCallable stays a kind test, [`is_callable`]) | M4-P4 PROXY1 |
//! | VALIDITY_CELL | 79 | a prototype's validity cell (V8 style; PROTO_INFO +32): a valid bit and a weak users list | M4-P1 VALIDITY (`validity.rs`) |
//! | WEAK_CELL | 80 | the weak target of a WeakRef or of a FinalizationRegistry entry (one weak trace function) | M4-P4 WEAKREF (`weakref.rs`) |
//!
//! No kind id is persisted (units, snapshots and facts never hold one), so the table may
//! change. A kind is registered by the module that owns its layout (`object::register_kinds`
//! calls them all). Strings use SEQ_LATIN1 (1) and SEQ_UTF16 (2) since P1 (H19).
//!
//! word0 flags (bytes 6-7): bits 0-7 are kind-specific (objects: the in-object slot count;
//! strings: bit 0 ATOM), bit 8 is-prototype, bit 9 is reserved for "hash present" (H12-Q1),
//! bits 10-15 are reserved.

// The table lists every id of v1, including ids whose kinds later work builds; this file is
// frozen after the week-4 scaffold, so it keeps the ids it does not use yet.
#![allow(dead_code)]

use ep9_heap::KindId;

const fn k(n: u8) -> KindId {
    match KindId::new(n) {
        Some(k) => k,
        None => panic!("kind 0 is reserved"),
    }
}

// ---- strings (1-7) ----
pub(crate) const SEQ_LATIN1: KindId = k(1);
pub(crate) const SEQ_UTF16: KindId = k(2);
pub(crate) const CONS: KindId = k(3);
pub(crate) const SLICED: KindId = k(4);
pub(crate) const THIN: KindId = k(5);
pub(crate) const EXT_LATIN1: KindId = k(6);
pub(crate) const EXT_UTF16: KindId = k(7);
/// Unused alias of [`SEQ_LATIN1`] from the week-3 layout. No code outside kinds.rs names it;
/// remove it when kinds.rs is next unfrozen.
pub(crate) const STRING: KindId = SEQ_LATIN1;

// ---- symbols (8) ----
pub(crate) const SYMBOL: KindId = k(8);

// ---- JS objects (16-63) ----
pub(crate) const OBJECT: KindId = k(16);
pub(crate) const FUNCTION: KindId = k(17);
pub(crate) const ARRAY: KindId = k(18);
pub(crate) const ERROR: KindId = k(19);
/// v0: the array and string iterator (%ArrayIteratorPrototype%, %StringIteratorPrototype%).
pub(crate) const ITERATOR: KindId = k(20);
pub(crate) const PRIMITIVE_WRAPPER: KindId = k(21);
pub(crate) const BOUND_FUNCTION: KindId = k(22);
/// A generator object (also async functions and async generators, by its mode field):
/// 8 fixed fields (`object::genf`). OBJECT_LAYOUT 2.
pub(crate) const GENERATOR: KindId = k(23);
/// An object with internal slots (Promise, Map, Date and the other ObjectClass kinds): fixed
/// field 0 is the ObjectClass id, fixed field 1 a separate cell with the internal slots (not
/// the in-object slots, which hold properties) (`object::slot_obj`). OBJECT_LAYOUT 2.
pub(crate) const SLOT_OBJECT: KindId = k(24);
/// An arguments object (§10.4.4): 2 fixed fields, the parameter map and the mapped count
/// (`object::argsf`). OBJECT_LAYOUT 3; its exotic hooks are in `arguments.rs`.
pub(crate) const ARGUMENTS: KindId = k(25);
/// A module namespace object (§10.4.6): 2 fixed fields, the module and the export names
/// (`object::nsf`). OBJECT_LAYOUT 3; its exotic hooks are in `module.rs`.
pub(crate) const MODULE_NAMESPACE: KindId = k(26);
/// A Proxy exotic object (§10.5) whose target is not callable. OBJECT_LAYOUT 4 (V9-D19);
/// built by M4-P4 PROXY1 (`proxy.rs`).
pub(crate) const PROXY: KindId = k(27);
/// A Proxy exotic object whose target is callable: `[[Call]]` (and `[[Construct]]` if the
/// target has it). A separate kind, so [`is_callable`] stays a kind test. OBJECT_LAYOUT 4.
pub(crate) const PROXY_CALLABLE: KindId = k(28);

// ---- internal cells (64-127) ----
pub(crate) const PROPS: KindId = k(64);
pub(crate) const ELEMENTS: KindId = k(65);
pub(crate) const CONTEXT: KindId = k(66);
pub(crate) const ACCESSOR_PAIR: KindId = k(67);
pub(crate) const FEEDBACK: KindId = k(68);
pub(crate) const POLY_IC: KindId = k(69);
pub(crate) const IC_HANDLER: KindId = k(70);
pub(crate) const SLOTS: KindId = k(71);
pub(crate) const DESCRIPTORS: KindId = k(72);
pub(crate) const DESC_INDEX: KindId = k(73);
pub(crate) const TRANSITIONS: KindId = k(74);
pub(crate) const PROTO_INFO: KindId = k(75);
/// A Source Text Module Record (§16.2.1.6), fixed size (`object::modf`). OBJECT_LAYOUT 3.
pub(crate) const MODULE: KindId = k(76);
/// Map and Set storage (an ordered hash table), variable size; registered by
/// `ordered_table.rs`. OBJECT_LAYOUT 3.
pub(crate) const ORDERED_TABLE: KindId = k(77);
/// A Realm Record (§9.3), fixed size (`object::realmf`). OBJECT_LAYOUT 3.
pub(crate) const REALM: KindId = k(78);
/// A prototype's validity cell (V9-D7; PROTO_INFO +32): invalid once the prototype or one
/// above it changes shape. OBJECT_LAYOUT 4; built by M4-P1 VALIDITY (`validity.rs`).
pub(crate) const VALIDITY_CELL: KindId = k(79);
/// The weak target of a WeakRef or a FinalizationRegistry entry (V9-D23). OBJECT_LAYOUT 4;
/// built by M4-P4 WEAKREF (`weakref.rs`).
pub(crate) const WEAK_CELL: KindId = k(80);

// ---- shapes (128) ----
pub(crate) const SHAPE: KindId = k(128);

// ---- word0 flags ----
/// Objects: the in-object slot count.
pub(crate) const FLAG_INOBJECT_MASK: u16 = 0x00FF;
/// Strings: the string is an atom (interned).
pub(crate) const FLAG_ATOM: u16 = 0x0001;
/// Objects: the object is a prototype (H17 prototype mode).
pub(crate) const FLAG_IS_PROTOTYPE: u16 = 0x0100;
/// Objects: the object has an identity hash (H12-Q1; set by `ordered_table::identity_hash`
/// when the object first becomes a Map, Set, WeakMap or WeakSet key).
pub(crate) const FLAG_HASH_PRESENT: u16 = 0x0200;

/// The cell test on raw bits (JSC `isCell`, H6): no number tag and no immediate tag bit.
#[inline(always)]
pub(crate) const fn is_cell_bits(bits: u64) -> bool {
    bits & ep9_value::encoding::NOT_CELL_MASK == 0
}

/// The kind byte of a cell value, or 0 for a non-cell.
#[inline(always)]
pub(crate) fn cell_kind(b: u64) -> u8 {
    if is_cell_bits(b) {
        // SAFETY: a cell Value held by the VM is live (crate invariant, object.rs).
        unsafe { crate::object::cell(b).header() }.kind()
    } else {
        0
    }
}

#[inline(always)]
pub(crate) fn is_string(b: u64) -> bool {
    matches!(cell_kind(b), 1..=7)
}

#[inline(always)]
pub(crate) fn is_symbol(b: u64) -> bool {
    cell_kind(b) == SYMBOL.get()
}

/// A JS object (kinds 16-63).
#[inline(always)]
pub(crate) fn is_object(b: u64) -> bool {
    matches!(cell_kind(b), 16..=63)
}

/// A FUNCTION cell (it has a FunctionInfo). Use [`is_callable`] for IsCallable.
#[inline(always)]
pub(crate) fn is_function(b: u64) -> bool {
    cell_kind(b) == FUNCTION.get()
}

/// IsCallable (§7.2.3): a FUNCTION, a BOUND_FUNCTION or a PROXY_CALLABLE (OBJECT_LAYOUT 4;
/// no code makes one before M4-P4 PROXY1).
#[inline(always)]
pub(crate) fn is_callable(b: u64) -> bool {
    let k = cell_kind(b);
    k == FUNCTION.get() || k == BOUND_FUNCTION.get() || k == PROXY_CALLABLE.get()
}

#[inline(always)]
pub(crate) fn is_array(b: u64) -> bool {
    cell_kind(b) == ARRAY.get()
}

#[inline(always)]
pub(crate) fn is_error(b: u64) -> bool {
    cell_kind(b) == ERROR.get()
}

#[inline(always)]
pub(crate) fn is_iterator(b: u64) -> bool {
    cell_kind(b) == ITERATOR.get()
}

#[inline(always)]
pub(crate) fn is_context(b: u64) -> bool {
    cell_kind(b) == CONTEXT.get()
}

#[inline(always)]
pub(crate) fn is_wrapper(b: u64) -> bool {
    cell_kind(b) == PRIMITIVE_WRAPPER.get()
}

#[inline(always)]
pub(crate) fn is_bound(b: u64) -> bool {
    cell_kind(b) == BOUND_FUNCTION.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_v1_ranges() {
        let strings = [
            SEQ_LATIN1, SEQ_UTF16, CONS, SLICED, THIN, EXT_LATIN1, EXT_UTF16,
        ];
        assert!(strings.iter().all(|k| (1..=7).contains(&k.get())));
        let objects = [
            OBJECT,
            FUNCTION,
            ARRAY,
            ERROR,
            ITERATOR,
            PRIMITIVE_WRAPPER,
            BOUND_FUNCTION,
            GENERATOR,
            SLOT_OBJECT,
            ARGUMENTS,
            MODULE_NAMESPACE,
            PROXY,
            PROXY_CALLABLE,
        ];
        assert!(objects.iter().all(|k| (16..=63).contains(&k.get())));
        assert_eq!((PROXY.get(), PROXY_CALLABLE.get()), (27, 28));
        assert_eq!((GENERATOR.get(), SLOT_OBJECT.get()), (23, 24));
        assert_eq!((ARGUMENTS.get(), MODULE_NAMESPACE.get()), (25, 26));
        let internal = [
            PROPS,
            ELEMENTS,
            CONTEXT,
            ACCESSOR_PAIR,
            FEEDBACK,
            POLY_IC,
            IC_HANDLER,
            SLOTS,
            DESCRIPTORS,
            DESC_INDEX,
            TRANSITIONS,
            PROTO_INFO,
            MODULE,
            ORDERED_TABLE,
            REALM,
            VALIDITY_CELL,
            WEAK_CELL,
        ];
        assert!(internal.iter().all(|k| (64..=127).contains(&k.get())));
        assert_eq!(
            (MODULE.get(), ORDERED_TABLE.get(), REALM.get()),
            (76, 77, 78)
        );
        assert_eq!((VALIDITY_CELL.get(), WEAK_CELL.get()), (79, 80));
        assert_eq!((CONS.get(), SLICED.get(), THIN.get()), (3, 4, 5));
        assert_eq!(SYMBOL.get(), 8);
        assert_eq!(SHAPE.get(), 128);
        let mut all: Vec<u8> = strings
            .iter()
            .chain(&objects)
            .chain(&internal)
            .map(|k| k.get())
            .collect();
        all.extend([SYMBOL.get(), SHAPE.get()]);
        let n = all.len();
        // OBJECT_LAYOUT 3: 30 ids before, 35 after; OBJECT_LAYOUT 4: 39.
        assert_eq!(n, 39);
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), n, "kind ids are distinct");
        assert_eq!(FLAG_INOBJECT_MASK & FLAG_IS_PROTOTYPE, 0);
        assert_eq!(FLAG_IS_PROTOTYPE & FLAG_HASH_PRESENT, 0);
    }
}
