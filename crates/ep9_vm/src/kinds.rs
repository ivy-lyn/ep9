//! Cell kind ids (word0 byte 5), table v1 (v2 of the object layout adds GENERATOR and
//! SLOT_OBJECT, M2-P0; v3 adds ARGUMENTS, MODULE_NAMESPACE, MODULE, ORDERED_TABLE and REALM,
//! M3-P0; v4 adds PROXY, PROXY_CALLABLE, VALIDITY_CELL and WEAK_CELL and the CONS, SLICED and
//! THIN layouts, M4-P0; v5 adds ELEMENTS_DOUBLE and ELEMENTS_DICT and reserves FUSE, M5-P0; v6 adds PROPS_INDEXED and the FUSE layout, M6-P0; v7 adds BIGINT, [`BIGINT`]; v9 adds the 12 TypedArray view kinds and BYTES, [`INT8_VIEW`], [`BYTES`]),
//! and the kind tests on Value bits.
//!
//! | Ids | Kinds | Range test |
//! |---|---|---|
//! | 1-7 | strings: SEQ_LATIN1, SEQ_UTF16; CONS, SLICED, THIN (layouts below, OBJECT_LAYOUT 4; built by M4-P1 STR1 and M4-P2 STR2); EXT_LATIN1, EXT_UTF16 reserved | [`is_string`] |
//! | 8; 9 | SYMBOL; BIGINT (OBJECT_LAYOUT 7, a single-id row: 10-15 stay free and in no range) | [`is_symbol`]; [`is_bigint`] |
//! | 16-63 | JS objects: OBJECT, FUNCTION, ARRAY, ERROR, ITERATOR, PRIMITIVE_WRAPPER, BOUND_FUNCTION, GENERATOR, SLOT_OBJECT, ARGUMENTS, MODULE_NAMESPACE, PROXY, PROXY_CALLABLE; 29-40 the 12 TypedArray view kinds in spec Table 71 order, INT8_VIEW .. FLOAT64_VIEW (OBJECT_LAYOUT 9, [`INT8_VIEW`]); 41-63 reserved | [`is_object`]; the views [`is_view_kind`], of them [`is_bigint_view_kind`] (36-37) and [`is_float_view_kind`] (38-40) |
//! | 64-127 | internal cells: PROPS, ELEMENTS, CONTEXT, ACCESSOR_PAIR, FEEDBACK, POLY_IC, IC_HANDLER, SLOTS, DESCRIPTORS, DESC_INDEX, TRANSITIONS, PROTO_INFO, MODULE, ORDERED_TABLE, REALM, VALIDITY_CELL, WEAK_CELL, ELEMENTS_DOUBLE, ELEMENTS_DICT, PROPS_INDEXED (84, OBJECT_LAYOUT 6), BYTES (85, OBJECT_LAYOUT 9, [`BYTES`]); FUSE (83) reserved, its layout declared (OBJECT_LAYOUT 6) | - |
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
//! OBJECT_LAYOUT 5 (M5-P0; plan v10 §2.3, V10-D2). The month-5 ids; P0 declares them and
//! registers none (the role that first allocates a kind registers its trace function):
//!
//! | Kind | Id | Layout (byte offsets) | Builder |
//! |---|---|---|---|
//! | ELEMENTS_DOUBLE | 81 | +8 capacity (an int32 Value), +16 raw f64 data (not Values: the trace function visits no data word); the hole is the NaN 0xFFF7_FFFF_FFF7_FFFF and every other NaN is stored as the canonical NaN (V9-D39, V10-D7; `object::delems`) | M5-P1 ELEM1 (`elements.rs`) |
//! | ELEMENTS_DICT | 82 | sparse elements: index -> (value or ACCESSOR_PAIR, attributes); also the elements with non-default attributes, accessor elements and a non-writable `length` (H18-Q2, Q4; the H17 "Element accessors" row) | M5-P2 ELEM2 (`elements.rs`) |
//! | FUSE | 83 | reserved: registered by M5-P1 FR0 (`fuse.rs`) only if a fuse word needs a cell (H31-Q1); else it stays reserved | M5-P1 FR0 |
//!
//! **The kind ids are append-only (C-24, V10-D5; decided M5-P0, H17 text by M5-P3 M1A-CODE).**
//! No id is renumbered or reused: a renumbering is an OBJECT_LAYOUT bump, and native code and
//! unit text bake kind ids (`crate::layout::KIND_*`). [`is_callable`] stays three compares
//! (FUNCTION, BOUND_FUNCTION, PROXY_CALLABLE): making the callable kinds adjacent would
//! renumber. `exotic::is_routed` stays the one range test 25..=28 and that range is closed: a
//! new kind with routed internal methods gets its own compare, never a renumbered id. A
//! retired id stays reserved. Negative tests probe 255, an id that is never assigned (never "the
//! next free id"), and no test or text tests a kind by a range other than the ones above.
//!
//! No kind id is persisted (units, snapshots and facts never hold one), but native code and
//! unit text bake them, so they follow the append-only rule above. A kind is registered by the
//! module that owns its layout (`object::register_kinds` calls them all). Strings use
//! SEQ_LATIN1 (1) and SEQ_UTF16 (2) since P1 (H19).
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
/// Raw f64 elements (H18-Q1, V9-D39): +8 capacity (int32), +16 f64 data, the hole NaN
/// (`object::delems`). OBJECT_LAYOUT 5; built by M5-P1 ELEM1 (`elements.rs`).
pub(crate) const ELEMENTS_DOUBLE: KindId = k(81);
/// Dictionary elements (H18-Q2, Q4): index -> (value or ACCESSOR_PAIR, attributes).
/// OBJECT_LAYOUT 5; built by M5-P2 ELEM2 (`elements.rs`).
pub(crate) const ELEMENTS_DICT: KindId = k(82);
/// A realm's static fuse block (H31-Q1, Q5; V11-D21): reserved in OBJECT_LAYOUT 5; its layout is
/// OBJECT_LAYOUT 6 (the table at [`PROPS_INDEXED`]); M6-P2 FUSE-PI registers and builds it.
pub(crate) const FUSE: KindId = k(83);

// ---- shapes (128) ----
pub(crate) const SHAPE: KindId = k(128);

// ---- internal cells of OBJECT_LAYOUT 6 (M6-P0) ----
// Here, after SHAPE, so the id lines above keep their line numbers (H17 "layout core" cites
// them by line, and that part is Frozen).

/// The property table of a dictionary-mode object with more than 8 entries (Dictionary v1, H17;
/// V11-D18). OBJECT_LAYOUT 6 (M6-P0; plan v11 §2.3) declares it and the FUSE layout, and
/// registers PROPS_INDEXED with its size and trace functions (`props_index.rs`):
///
/// | Kind | Id | Layout (byte offsets) | Builder |
/// |---|---|---|---|
/// | PROPS_INDEXED | 84 | +8 capacity, +16 count (int32 Values; count = the live entries), +24 index (an ELEMENTS 65 cell: +16 `used`, int32 buckets from +24 (2 x capacity of them); undefined when 8 or fewer entries are live), +32 entries: key, value, attributes (as PROPS 64, which does not change); an object moves from PROPS to it above 8 entries | M6-P1 DICT1 (`props_index.rs`) |
/// | FUSE | 83 | a realm's static fuse block: from +8 one u32 per entry of `fuse::STATIC` (entry `i` at +8 + 4i; 1 intact, 0 popped), raw words that the GC does not trace; size 8 + 4 x entries, rounded up to 8 (`layout::FUSE_*`); the realm holds it as `IntrinsicId::RealmFuses` and `Root::RealmFuses` mirrors it | M6-P2 FUSE-PI (`fuse.rs`) |
pub(crate) const PROPS_INDEXED: KindId = k(84);

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

/// IsCallable (§7.2.3): a FUNCTION, a BOUND_FUNCTION or a PROXY_CALLABLE (OBJECT_LAYOUT 4).
/// Three compares on purpose (C-24): one range test would need adjacent ids, a renumbering.
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

// ---- the BigInt primitive of OBJECT_LAYOUT 7 (spec-temporal B2) ----
// Here, after the kind tests, so no line above moves (H17 cites them by line).

/// A BigInt value (§6.1.6.2): a primitive cell, ShapeId 0 as strings and symbols, word0 flags
/// 0; +8 u32 digit count (bits 0-30) and sign (bit 31), +12 u32 hash, +16 u64 digits
/// (`bigint.rs`). Its own single-id row 9 of the table above ([`is_bigint`]); ids 10-15 stay
/// free and in no range. OBJECT_LAYOUT 7.
pub(crate) const BIGINT: KindId = k(9);

/// A BigInt (kind 9, OBJECT_LAYOUT 7).
#[inline(always)]
pub(crate) fn is_bigint(b: u64) -> bool {
    cell_kind(b) == BIGINT.get()
}

// ---- the TypedArray view kinds and BYTES of OBJECT_LAYOUT 9 (TypedArray TA-0, TA-DECL) ----
// At the end of the const block, after the BIGINT test above, so no line of the table and of
// the older ids moves (H17 cites them). Design `docs/plan/notes/typedarray-design.md` §2.1
// (TA-D1, TA-D4). Declared only: TA-1 TA-VM registers them (`object::register_kinds`) and
// builds their cells.

/// A view of an Int8Array (spec Table 71 row 1): the first of the 12 view kinds, 29-40, one
/// per element type, adjacent and in Table 71 order, so the element type of a view is its kind
/// minus [`INT8_VIEW`] (TA-D1). Each view kind is a JS object with 4 fixed fields: +24 buffer,
/// +32 data (an `ExternalRef<u8>` native word), +40 length, +48 byteOffset; the in-object slots
/// follow at +56 (`layout::VIEW_*`). Its elements store (+16) is always undefined.
/// OBJECT_LAYOUT 9.
pub(crate) const INT8_VIEW: KindId = k(29);
/// Uint8Array (Table 71 row 2). OBJECT_LAYOUT 9.
pub(crate) const UINT8_VIEW: KindId = k(30);
/// Uint8ClampedArray (Table 71 row 3). OBJECT_LAYOUT 9.
pub(crate) const UINT8_CLAMPED_VIEW: KindId = k(31);
/// Int16Array (Table 71 row 4). OBJECT_LAYOUT 9.
pub(crate) const INT16_VIEW: KindId = k(32);
/// Uint16Array (Table 71 row 5). OBJECT_LAYOUT 9.
pub(crate) const UINT16_VIEW: KindId = k(33);
/// Int32Array (Table 71 row 6). OBJECT_LAYOUT 9.
pub(crate) const INT32_VIEW: KindId = k(34);
/// Uint32Array (Table 71 row 7). OBJECT_LAYOUT 9.
pub(crate) const UINT32_VIEW: KindId = k(35);
/// BigInt64Array (Table 71 row 8): the first of the 2 BigInt views ([`is_bigint_view_kind`]).
/// OBJECT_LAYOUT 9.
pub(crate) const BIGINT64_VIEW: KindId = k(36);
/// BigUint64Array (Table 71 row 9). OBJECT_LAYOUT 9.
pub(crate) const BIGUINT64_VIEW: KindId = k(37);
/// Float16Array (Table 71 row 10): the first of the 3 float views ([`is_float_view_kind`]).
/// OBJECT_LAYOUT 9.
pub(crate) const FLOAT16_VIEW: KindId = k(38);
/// Float32Array (Table 71 row 11). OBJECT_LAYOUT 9.
pub(crate) const FLOAT32_VIEW: KindId = k(39);
/// Float64Array (Table 71 row 12): the last view kind. OBJECT_LAYOUT 9.
pub(crate) const FLOAT64_VIEW: KindId = k(40);

/// Raw bytes (internal, TA-D4): the store of a fixed-length, non-shared ArrayBuffer. +8 the
/// byte length (a number Value), +16 the bytes, zeroed at birth (8-aligned; no Value, so the
/// trace function visits no word, as ELEMENTS_DOUBLE). Above 8 KiB the cell is a large object.
/// OBJECT_LAYOUT 9.
pub(crate) const BYTES: KindId = k(85);

/// The number of view kinds: one per row of spec Table 71.
pub(crate) const VIEW_KINDS: u8 = FLOAT64_VIEW.get() - INT8_VIEW.get() + 1;
const _: () = assert!(VIEW_KINDS == 12);

/// True for the 12 view kinds 29-40 (one range compare, TA-D6). Not routed: the range 25..=28
/// of `exotic::is_routed` stays closed, so a view kind gets this compare of its own.
#[inline(always)]
pub(crate) const fn is_view_kind(k: u8) -> bool {
    k >= INT8_VIEW.get() && k <= FLOAT64_VIEW.get()
}

/// True for the 2 BigInt views, BigInt64Array and BigUint64Array (36-37): their elements are
/// BigInts (`[[ContentType]]` bigint).
#[inline(always)]
pub(crate) const fn is_bigint_view_kind(k: u8) -> bool {
    k >= BIGINT64_VIEW.get() && k <= BIGUINT64_VIEW.get()
}

/// True for the 3 float views, Float16Array, Float32Array and Float64Array (38-40).
#[inline(always)]
pub(crate) const fn is_float_view_kind(k: u8) -> bool {
    k >= FLOAT16_VIEW.get() && k <= FLOAT64_VIEW.get()
}

/// A TypedArray (a cell of one of the 12 view kinds, [`is_view_kind`]).
#[inline(always)]
pub(crate) fn is_view(b: u64) -> bool {
    is_view_kind(cell_kind(b))
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
            ELEMENTS_DOUBLE,
            ELEMENTS_DICT,
            FUSE,
            PROPS_INDEXED,
        ];
        assert!(internal.iter().all(|k| (64..=127).contains(&k.get())));
        assert_eq!(
            (MODULE.get(), ORDERED_TABLE.get(), REALM.get()),
            (76, 77, 78)
        );
        assert_eq!((VALIDITY_CELL.get(), WEAK_CELL.get()), (79, 80));
        assert_eq!(
            (ELEMENTS_DOUBLE.get(), ELEMENTS_DICT.get(), FUSE.get()),
            (81, 82, 83)
        );
        assert_eq!(PROPS_INDEXED.get(), 84);
        assert_eq!((CONS.get(), SLICED.get(), THIN.get()), (3, 4, 5));
        assert_eq!(SYMBOL.get(), 8);
        assert_eq!(BIGINT.get(), 9);
        assert_eq!(SHAPE.get(), 128);
        let mut all: Vec<u8> = strings
            .iter()
            .chain(&objects)
            .chain(&internal)
            .map(|k| k.get())
            .collect();
        // BIGINT (9) is its own range of one id (OBJECT_LAYOUT 7).
        all.extend([SYMBOL.get(), BIGINT.get(), SHAPE.get()]);
        let n = all.len();
        // OBJECT_LAYOUT 3: 30 ids before, 35 after; OBJECT_LAYOUT 4: 39; OBJECT_LAYOUT 5: 42;
        // OBJECT_LAYOUT 6: 43; OBJECT_LAYOUT 7: 44.
        assert_eq!(n, 44);
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), n, "kind ids are distinct");
        assert_eq!(FLAG_INOBJECT_MASK & FLAG_IS_PROTOTYPE, 0);
        assert_eq!(FLAG_IS_PROTOTYPE & FLAG_HASH_PRESENT, 0);
    }

    /// OBJECT_LAYOUT 9 (TypedArray TA-0): the 12 view kinds at 29-40 in spec Table 71 order and
    /// BYTES 85, with their range tests; together with the ids of `table_v1_ranges` (44), 57
    /// distinct ids. A view is a JS object (16-63) and no routed kind (25..=28 stays closed).
    #[test]
    fn table_v9_view_kinds() {
        let views = [
            INT8_VIEW,
            UINT8_VIEW,
            UINT8_CLAMPED_VIEW,
            INT16_VIEW,
            UINT16_VIEW,
            INT32_VIEW,
            UINT32_VIEW,
            BIGINT64_VIEW,
            BIGUINT64_VIEW,
            FLOAT16_VIEW,
            FLOAT32_VIEW,
            FLOAT64_VIEW,
        ];
        assert_eq!(
            views.map(|k| k.get()),
            core::array::from_fn(|i| 29 + i as u8)
        );
        assert!(views.iter().all(|k| (16..=63).contains(&k.get())));
        assert_eq!(BYTES.get(), 85);
        assert!((64..=127).contains(&BYTES.get()));
        let in_range = |t: fn(u8) -> bool| (0..=u8::MAX).filter(|&k| t(k)).collect::<Vec<_>>();
        assert_eq!(in_range(is_view_kind), (29..=40).collect::<Vec<u8>>());
        assert_eq!(in_range(is_bigint_view_kind), [36, 37]);
        assert_eq!(in_range(is_float_view_kind), [38, 39, 40]);
        assert!(!(25..=28).any(is_view_kind), "no routed kind is a view");
        // The ids of table v1 (strings, symbol, BigInt, JS objects 16-28, internal 64-84, shape).
        let mut all: Vec<u8> = (1..=9)
            .chain(16..=28)
            .chain(64..=84)
            .chain([128])
            .chain(views.map(|k| k.get()))
            .chain([BYTES.get()])
            .collect();
        let n = all.len();
        assert_eq!(n, 44 + 13);
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), n, "kind ids are distinct");
    }
}
