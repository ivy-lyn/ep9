//! Versions of the VM-internal contracts that native code (JIT, AOT units) bakes in. An AOT
//! unit records them in its header (H35); a unit whose versions differ from the running
//! VM's is not loaded as native code. Version 0 means "no native form yet". Bump a version
//! with every change to its contract; these are not the bytecode ABI (`ep9_bytecode`
//! `ABI_VERSION`).

/// Object layout: word0, the object header, fixed fields, slots (H8, H17). 2 (M2-P0): the
/// GENERATOR and SLOT_OBJECT kinds, the ITERATOR kind field, the CONTEXT scope word as a
/// native word (object.rs). 3 (M3-P0): the FUNCTION realm field (+48; the in-object slots
/// move to +56); the JS object kinds ARGUMENTS (25) and MODULE_NAMESPACE (26); the internal
/// cells MODULE (76), ORDERED_TABLE (77, with a weak-keys flag) and REALM (78). 4 (M4-P0):
/// the JS object kinds PROXY (27) and PROXY_CALLABLE (28, callable); the internal cells
/// VALIDITY_CELL (79) and WEAK_CELL (80); the PROTO_INFO +32 word holds the prototype's
/// validity cell (was reserved); the string cells CONS (3: +16 first, +24 second), SLICED (4:
/// +16 parent, +24 offset) and THIN (5: +16 actual), each after the string header of
/// `string.rs` (+8 length (u32), +12 hash field (u32)) (`kinds.rs` has the table). The kinds
/// are declared here; the roles that build them register their trace functions.
///
/// 5 (M5-P0; plan v10 §2.3, V10-D2: the one bump of month 5):
/// - the internal kinds ELEMENTS_DOUBLE (81: +8 capacity (int32), +16 raw f64 data, the hole
///   NaN 0xFFF7_FFFF_FFF7_FFFF, no data word traced) and ELEMENTS_DICT (82: sparse elements,
///   index -> (value or ACCESSOR_PAIR, attributes)); FUSE (83) reserved (`kinds.rs`); kind ids
///   are append-only (C-24, `kinds.rs`);
/// - the shape's elements kind in `counts` bits 28-31, with the values 0 PACKED_INT32,
///   1 HOLEY_INT32, 2 PACKED_DOUBLE, 3 HOLEY_DOUBLE, 4 PACKED_ELEMENTS, 5 HOLEY_ELEMENTS,
///   6 DICTIONARY (7-15 reserved, append-only; bit 0 = holey). Storage forms: {0, 1, 4, 5}
///   Values in an ELEMENTS cell, {2, 3} raw f64 in an ELEMENTS_DOUBLE cell, {6} an
///   ELEMENTS_DICT cell (`elements.rs`, `shape.rs`). The bits stay 0 and nothing reads them
///   until M5-P1 ELEM1 builds the lattice;
/// - the slack counter of a root shape in its `flags` word, bits 4-7 (`shape::flag`);
/// - the representation of a descriptor in details bits 4-6 (0 none, 1 int32, 2 double,
///   3 heap object, 4 any) and its const bit 7 (`shape.rs`; reserved and 0 before);
/// - ARRAY +32: the allocation-site handler (an IC_HANDLER cell of kind ALLOC_SITE, IC_LAYOUT
///   3) of an array made at an array-literal site, or undefined (V10-D6; `object::arr`);
/// - REALM +48 and +56: the cached shapes of mapped and unmapped arguments objects, or
///   undefined (V10-D11; `object::realmf`, the cell grows from 48 to 64 B);
/// - named here, unchanged since 4 (MC-w16-19): PROXY and PROXY_CALLABLE keep `[[ProxyTarget]]`
///   at +24 and `[[ProxyHandler]]` at +32 (fixed fields 0 and 1, null once revoked;
///   `proxy.rs`); WEAK_CELL (80) is +8 target (weak), +16 held value (strong), +24 unregister
///   token (weak), +32 next cell (strong), 40 B (`weakref.rs`).
pub const OBJECT_LAYOUT_VERSION: u32 = 5;
/// Inline-cache and feedback data layout (H16). 2 (M4-P0): the feedback slot table of
/// `profile.rs` (one writer module per slot kind) and the IC_HANDLER layout of `ic.rs` (+8
/// handler kind, +16 validity cell or int32, +24 weak ref, +32 int32; new handler kinds
/// CONSTRUCT_SITE, INSTANCEOF and CLONE).
pub const IC_LAYOUT_VERSION: u32 = 2;
/// The frame layout: 8-slot header, arguments, registers (H14). 2 (M2-P0): fp-3 holds
/// new.target, or the generator object in a resumed frame (`frame::slot::NEW_TARGET`).
pub const FRAME_VERSION: u32 = 2;
/// The builtin (host function) ABI (H20).
pub const BUILTIN_ABI_VERSION: u32 = 1;
/// Deoptimization metadata (H29): none exists yet.
pub const DEOPT_META_VERSION: u32 = 0;
/// The VM layout (C-18, doc 13 C-18; MC-w15-5: one owner per fact): the layout facts native
/// code bakes in that no other version owns. They are the interpreter `Cx` field offsets
/// (`layout::CX_*`), the FunctionInfo offsets (`layout::FI_*`, `FI_FEEDBACK` included) and the
/// RuntimeTable offsets (`layout::RT_*`) with the table's length. 1 (M5-P0): `Cx` has no
/// `proto_epoch`, and `native_limit`, `runtime_table` and `unit_data` exist in every build (they
/// were `spike-native` only), so the `Cx` layout no longer depends on the features; the
/// RuntimeTable has 11 entries (entry 10 `deopt`, M5-P0). Frame offsets belong to
/// [`FRAME_VERSION`], cell and kind facts to [`OBJECT_LAYOUT_VERSION`], IC data to
/// [`IC_LAYOUT_VERSION`] and Value bits to the value encoding (`ep9_value`).
pub const VM_LAYOUT_VERSION: u32 = 1;
